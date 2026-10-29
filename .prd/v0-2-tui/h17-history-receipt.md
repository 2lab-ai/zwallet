# H17 — history and transaction detail

Status: in-progress, 2026-10-29. EVM implementation, independent review and the
parent full gate are complete. PR #45 is published; a final frame-assertion update
is under validation. This does not claim that all TUI workflows or Tron history shipped.

## Intent and scope

The fixed user requests in [ssot.md](ssot.md) are:

> 디폴트로 밸런스와 최근 10건 트랜잭션 리스트를 출력해줌
> 기존 tx를 선택하면 해당 내역을 제세히 출력해줌
> 트랜스퍼하고 tx번호로 확인할수 있도록 이를 간단한 tui로

TUI-7+8 / T3, T3b, T4, T6c and T26 map to the bounded history/detail path below.
The intent is to inspect the selected key's activity without turning an explorer or
node claim into a verified payee or signing authorization. Existing H6 parsers and
navigation are reused; immutable request state and rendering are the new glue.
Replacement signing remains TUI-9, with explicit unavailable controls here.

## Implemented boundary

- Wallet balances plus at most ten history rows, pending first; Enter opens detail,
  `/` accepts a complete EVM hash, and `tx` in the action menu works from other tabs.
  `Tab` changes balance/history focus. Early hash input survives the owner lookup.
- Journal/explorer source labels and bounded coverage notes remain visible. Only
  the three fixed Blockscout hosts are used. A failed explorer leaves journal rows.
- Detail includes original hash, node state, outer from/to, native value, nonce,
  receipt fee/L1 fee, confirmations and registry-only wallet token movements. Missing
  hashes retain the lookup hint; consumed journal nonce displays dropped/replaced.
  Unknown/missing `to` is not mislabeled as contract creation.
- Owner resolution precedes history fetch. Every result captures generation, key,
  chain, exact endpoint and owner; each detail additionally has a request sequence
  and hash. A switched key/chain/endpoint, changed owner or superseded hash cannot
  adopt a late response. The RPC detail backend rejects a provider for a different
  chain/endpoint before any RPC.
- Reads re-resolve the selected key from the passed home. Detail re-reads that
  owner's journal rather than trusting a caller-supplied nonce row. Nicknames require
  the password-checked list and unchanged file fingerprint; same-generation byte
  rewrites drop the list. Node names, methods and token symbols are never labels.
- All outside content uses bounded inert text. TestBackend rendering goes through
  FilteredBackend with zero replacements and checked placement rectangles at 80x24;
  hostile errors remain within the error budget and at most seven rows.

## Stable extension API

`src/tui/app/history.rs` exposes `HistoryTarget`, `HistoryScope`, `DetailRequest`,
`HistoryRead`, `DetailRead`, `TransactionDetail` and `App::history()`. The request's
`journal` and explorer flag are provenance/context only, not proof of an original
signing intent. `Msg::Transaction` and `Effect::LoadTransaction` box `DetailRequest`.
`src/tui/ui/history.rs` draws the history/detail pane. `history_key` is the TUI-9
extension point; its current s/x/R handling returns an unavailable notice.

`JournalRow::nonce` is optional and `expiration_ms` is present for the Tron adapter.
`TransactionDetail::Tron(TronDetail)` has an unprefixed hash, no nonce, expiry,
fee/resources and an expired-unseen state with a second-payment caution. This branch
still has the EVM-only chain registry, so the Tron model is not a reachable lookup.
When C is integrated, TUI-12 must:

1. Add Tron journal shape validation (exact base58 owner, 64-hex hash, no nonce,
   RFC3339 expiresAt converted to milliseconds) and journal-only history coverage.
2. Select the Tron hash parser/field for its chain and call
   `tron::lookup::lookup_tx(net, hash, expiration_ms)`; decode only known hash-verified
   fields, preserving unknown execution outcomes and expired-unseen as a claim.
3. Keep Tron replacement controls unavailable and never infer safe retry from
   expiry or absence. EVM fee/nonce/receipt parsing must not run on Tron.

## Observed verification, 2026-10-29

Dedicated target directory: `target-handoff-tui-history`, Rust 1.95.

| Acceptance | Observed checks | Result |
|---|---|---|
| T3/T3b | 14 rows, deduped journal/explorer hash, ten rendered rows, pending first, unavailable/403 coverage and forged explorer strings | passed in `tests/tui_history.rs` and `tests/history.rs` |
| T4 | fee/L1/confirmations/registry receipt movements, hidden movement count, dropped/malformed node data, bounded hostile errors, checked nickname fingerprint | passed in UI/backend/unit tests |
| T6c | consumed journal nonce becomes dropped/replaced, original hash/nonce remain shown, no safe-retry claim | passed in UI/backend tests |
| T26 | 63-digit refusal, 64-digit lookup, not found, stale request/chain/key/endpoint rejection, changed owner, tx action outside wallet | passed in UI tests |
| TUI-3+4 regression | thirteen pure navigation `nav_` tests | passed |

Commands and results:

- `cargo test --locked --test tui_history`: **10 passed**.
- `cargo test --locked --test tui navigation::nav_`: **13 passed**, 43 filtered out.
- `cargo test --locked --lib tui::app::history::tests`: **1 passed**.
- `cargo test --locked --test history`: **19 passed, 1 live test ignored**.
  Local loopback fixture sockets required the sandbox's approved escalation; no
  external node, wallet or daemon was contacted.
- `cargo fmt --check`, `git diff --check` and
  `cargo clippy --locked --all-targets -- -D warnings`: **passed**.

Observed RED→GREEN: before the history renderer, T3's frame still showed the old
wallet view and lacked recent rows. The additional tx-menu regression failed because
no `HistoryOwner` effect was emitted; reloading the wallet and preserving early hash
input made the same test pass. Prior pending-field review fixes retain tests for null
position/timestamp/fee on pending rows and strict mined/malformed shapes.

No full CLI/PTY suite, real wallet, daemon, live explorer or broadcast ran for H17.
The dispatcher owns full-suite/PTY/Linux gates and the combined Tron integration.

## Standalone PR39 integration — 2026-10-29

- Ready tree: `.worktrees/handoff-tui-history-pr`, branch `handoff/tui-history-pr`, based on
  `f0b2b25ffb73648a47886ec97f1db05abc1e8c43` (PR39). Applied the final backend patch followed by
  the UI delta; no old navigation snapshot was copied over the PR39 source.
- Preserved PR39 navigation, CR password submissions, activation failure reporting through
  `fail_pane`, and the single PasswordBuf ZeroizeOnDrop marker. README, password implementation,
  existing `tests/tui.rs`, CLI handlers and wallet read code have no diff from PR39. H13 send,
  H15 tools, H16 creation and reachable Tron behavior are not part of this standalone change.
- Reused `/Users/dd/Documents/Codex/2026-10-29/task/zwallet/target-handoff-tui-navigation`, after
  `cargo +1.95.0 clean -p zwallet`. No new build target was created.
- On this exact tree: `tui_history` 10 passed; navigation Msg/TestBackend 13 passed; checked
  history-fingerprint library regression 1 passed; history backend 19 passed, 1 live test ignored.
  The first sandboxed backend run was refused at localhost listener creation; the approved
  loopback-only rerun passed. No external RPC, explorer, daemon or real wallet was contacted.
- `cargo +1.95.0 clippy --locked --all-targets -- -D warnings`, formatting and diff checks
  passed. No CLI suite, PTY suite, or whole test suite ran for this integration.
- Complete standalone patch: `/tmp/zwallet-tui-history-pr-final.patch`, including new files,
  against the PR39 commit above. All changes remain uncommitted for the parent-owned whole
  gate, commit, push and CI. Original author source and input patches remain unchanged.

## Parent whole-gate acceptance

The standalone PR39-based integration passed Rust1.95.0 fmt --check, locked all-target Clippy
with warnings denied and cargo test --locked: **598 passed, 0 failed, 12 explicit live skips**.
Existing navigation/PTY regressions remain intact. The fixture's live Blockscout check stays an
explicit ignored network gate; synthetic fixture data is not claimed as a captured live response.

The first whole gate exposed a test-server shutdown hang in the oversized-response case.
The bounded client rejected Content-Length before draining the body, while the helper's blocking
write and synchronous Drop join prevented current-thread runtime cleanup from closing the socket.
Only the test server now has a bounded write timeout. The existing transport regression passed
again (0.13s), then the whole gate above passed. Production HTTP body/deadline limits are unchanged.
Logs: /tmp/zwallet-history-transport-fix.log, /tmp/zwallet-tui-history-final-gate.log.
Exact-head Linux/macOS CI remains required. No real wallet or funds were used.

## Parent verification and PR delivery, 2026-10-29

The parent ran Rust 1.95.0 fmt, all-target strict Clippy and the full locked suite:
598 passed, zero failed, 12 explicit live-test skips. This includes real isolated
CLI/PTY fixtures; no real wallet or signed public broadcast was used.
The initial oversized-body test server could block writing after client rejection
and then block its Drop join; a test-only 200ms write timeout removes that hang.
Commit `9addb25b04b54ea0ef2405b3ed5b5a4db6d134f8` is published as
[PR #45](https://github.com/2lab-ai/zwallet/pull/45). Its successor strengthens every
history frame assertion with single ASCII cells and the exact style allowlist,
in addition to existing source budgets, placement and zero replacement checks.
The same whole gate is repeated before committing that test-only update.
