# H18 — Tron receive, balances, send and transaction lookup

Status: in-progress — implementation, focused mock gates and independent final review complete;
whole repository gate, actual PTY execution and publication remain with the parent.
Measured: 2026-10-29. No real funds, funded public broadcast or TR23 acceptance was attempted.

## Intent and scope

The delegated TUI-12 scope is T38–T41 of 03-tui.md: the existing tabs work with Tron values,
immutable payment review and explicit uncertainty. H18A reuses C's validated transaction pipeline;
H18B adapts H17's read-only history. No Tron call, invoke, message signing or nonce replacement is added.

## Integrated boundaries

- H13 `685a363` supplies verified key/index pins, shared send ownership, terminal failure drain,
  q-error preservation and the connect-inclusive receipt deadline.
- C `1a4f36f` supplies strict codec/policy, dynamic conservative cost bounds, exact raw/signature
  verification, fsynced expiration journal and tri-state/expired-unseen lookup. Its lookup.rs and
  tests/tron_send.rs remain byte-identical to that validated commit.
- H17 `18b2f8e` supplies history/detail and its latest display assertions. Explorer mock write
  timeout and latest EVM detail source are preserved exactly. H18B adds family-safe address/trust
  display, Tron journal parsing and raw-hash-checked detail.
- Shared read APIs preserve CLI activation/owner warnings, per-row missing Tron cache errors and
  address repair additions. `NetworkRead`/`AssetRead` carry details while tuple wrappers preserve
  existing send consumers.
- SendStatus uses `nonce: Option<u64>` and `expiration: Option<u64>`; the EVM Progress event still
  carries a plain u64 nonce. H19 integration must gate replacement actions on EVM + Some(nonce).
  Direct receipt jobs pass `expiration: None` for EVM. The H15 deferred-lock return effects are retained.

## Observed focused gates

All tests below use isolated loopback HTTP/Unix sockets or pure state; no CLI/PTy suite was executed
by this work unit. Target: `target-handoff-tron-codec` after package clean when changing worktrees.

| Mapping | Observation |
|---|---|
| T38 | Tron before-open rows/footer never show an EVM primary address; receive retains password family pin. H18B trust tests cover cached/session/password/locked/impostor states. |
| T39 | Actual shared network read feeds TRX/USDT balances, activation status and owner warning. Default four-row balance view says do not fund; Tab + j/k exposes the complete warning. |
| T40 | Both assets traverse App assets/payee/amount/review/exact nickname, signer, fsynced journal and broadcast. Fee rise returns an unsigned new review; lowered maximum can sign. Key/index/list/lock changes refuse. Actual receipt worker distinguishes success/failure/missing result; first-observation deadline stays unknown. |
| T40 q/L | Submitted changed-review, success and malformed-signature jobs drain before deferred LockAll and quit; signer errors remain failing results. Repeat override warning remains visible during typed confirmation. |
| T41 | App slash accepts bare/prefixed txID, real mock lookup checks raw hash and displays full USDT contract, exact T addresses, fee sun and expiry. Missing execution remains unknown. History is explicitly local-journal-only; s/x/R stay unavailable. |
| Display integrity | Every 80x24 integration frame checks cell characters, independent style allowlist, source budgets and exact placement. H18B also verifies the small-terminal fallback. |

Observed counts: tron_tui **11**, tron_history **20**, tron_review **8**, transfer_review **21**,
tui_history **10**, tui_transfer_deadline **1** passed. Strict all-target Clippy passed; final fmt and
Clippy were repeated after the last reviewed token-row correction and passed.

RED→GREEN observations include fractional fee bounds rendered as `<invalid>`, clipped second-payment
and owner warnings, first wait timeout missing the second-payment caution, and full registry token
address absent from detail. Tests now assert the actual numeric/text contract, not only labels.

## Process acceptance prepared for parent

`tests/tron_tui_pty.rs::t38_t41_real_tron_receive_balance_send_and_hash_lookup_for_both_assets`
compiles. It uses real newly generated isolated wallets/daemon plus loopback mainnet-shaped nodes,
CR input, 80x24 terminal parsing, exact nickname, both assets, journal-before-broadcast proof,
receipt, bare-hash lookup, no secret echo and restored terminal modes. **Not executed here.**
The parent owns its serial run and complete fmt/Clippy/cargo-test gate before commit or PR.

## Remaining integration mechanics

The worktree retains the pending merge of C `deda723` onto H13; latest C fixes were copied exactly.
H17latest source was merged by file so H13 terminal and send changes were preserved. The parent may
record latest C/H17 ancestry when committing. No commit or push was made by this work unit.

## Independent final review

The independent reviewer re-read the final source and directly ran the six focused integration
binaries above (71 tests), `--lib tui::terminal_send_failure_tests` (5) and `--lib quit_error_tests`
(2): **78 passed, no remaining confirmed finding**. Its clean separate target used Rust 1.95.0,
`--locked --offline`; all 92 src/test SHA256 values matched before and after those runs.
Snapshot manifest SHA256: `3d1c12606e5e0eb1517665f25b849de55324bdd6b675678a0ce7f011b28b58d8`.
The reviewer verified fractional fee bounds, repeat-payment warning, immediately visible owner
warning, full token contract, exact family/key binding, no higher confirmed cost, journal before
broadcast, no nonce, original deadline and queued q/L error preservation. It ran no actual PTY,
real daemon or public-node request. Full process/repository gates remain parent-owned.

## Numeric-bound and PTY-fixture follow-up

The parent first actual PTY attempt stopped at CLI argument parsing: the test put global network
flags before the `tui` subcommand. The fixture now uses `tui --chain tron --rpc ...`; no product
CLI behavior changed. It compiles; actual parent rerun is pending.

A real App/TestBackend max-USDT regression reproduced a clipped formatted amount that hid the
asset and six-decimal scale. The complete 78-digit integer was already present; the review now
explicitly labels it as exact base units with USDT and six decimals. Short values also retain their
formatted decimal; long values use the complete integer with its explicit scale instead of a cut
decimal. MAX and MAX-1 differ visibly through their final base unit in both review and typed
confirmation; decoding matches each complete value and SignTx remains zero. TRX is bounded to
int64 sun (19 digits), so its formatted amount fits the normal row.

`tron_tui` now has **12 passing tests**, including
`t40_maximum_usdt_exact_base_units_keep_asset_and_decimals_visible`.
Final follow-up fmt/all-target Clippy and PTY compile-only checks passed. The original independent
78-test receipt above remains tied to its earlier manifest; follow-up has a separate source
manifest and narrow independent review.

## Tron action-menu coverage follow-up

The final 55-action inventory found four declared aliases still marked unimplemented. Two App
regressions first reproduced that disabled state. `tron address`, `tron balance`, `tron transfer`
and `tron tx` now delegate to the ordinary family-aware handlers, requiring the explicit Tron
chain selection specified by 05-tron.md §TUI. On EVM the reason is `select Tron with c`; selecting
an alias never switches the endpoint or key silently and never starts a signature.

The App menu tests check each actual effect, current chain/endpoint/key, full T-address scope,
TRX asset picker or bare txID request, and zero SignTx/broadcasts. Final focused Tron UI count is
**14 passed**; Tron history20 and EVM history10 also passed, followed by strict all-target Clippy/fmt.

## Late-read error preservation follow-up

Parent H7 independent RED showed that, after a submitted send failed during q + deferred L,
a same-generation Keys/Status read failure replaced the original broadcast error and its
recovery details. `fail_pane` now retains the first quit error with `get_or_insert`. The supplied
public-App regression `tests/tui_late_read_error.rs` passed all **3** tests here: late Keys, late
Status, and obsolete-generation/no-lock boundaries. This test starts no worker, socket, daemon,
CLI, or real wallet. The fix does not expand signing authority.

## Parent actual process acceptance, 2026-10-29

The parent executed the real isolated-daemon/PTy T38–T41 case for both TRX and
registry USDT: password-derived receive address, balances, exact review/nickname,
signed loopback broadcast, journal-before-broadcast, mined receipt/expiration and
bare-hash history detail all passed. The first attempts exposed only fixture CLI
arguments: chain belongs after `tui`, and TUI has no `--rpc` option. The fixture
now supplies the explicit loopback URL through ZWALLET_RPC_TRON, as the real TUI
startup contract requires. Final observed result: 1 test passed (both assets),
zero failures, 10.25 seconds. Log: /tmp/zwallet-h18-pty-third-run.log.

The source also contains the reviewed MAX/MAX-1 exact quantity/asset display, all
four current-Tron menu aliases, and common late-read quit-error preservation.
The full Rust gate follows these changes before commit/publication. No real
wallet, external signing request or funded/public broadcast was performed.

## Header family/trust matrix follow-up

The first whole gate exposed a stale fixture that sent an EVM primary status address for every
chain. The fixture now supplies the separate family Address observation for Tron while retaining
the 32-character key-name, every-chain and every-trust matrix. With that correct input it exposed
a real 80-column overflow: displaying all 34 Tron characters clipped the impostor warning.
Only the status-bar address formatter now uses first six / last four characters for both families.
Receive, reviewed payees and full-address detail retain the complete address.

The corrected matrix passed, followed by Tron history 20/20 and Tron UI 14/14 tests, strict
all-target Clippy and fmt. No actual PTY or whole gate was repeated by the author; the parent
owns the fresh whole gate. The alias regression now checks the shortened status-bar address
and still refuses an EVM address under a Tron label.

## Parent final full gate — 2026-10-29

After the header correction, the parent directly reran Rust 1.95.0 fmt, strict
locked all-target Clippy and the complete locked debug test suite: **764 passed,
0 failed, 15 ignored**. The suite again included the actual TRX/USDT PTY plus
all EVM transfer, navigation, history, deadline and daemon integration tests.
Log: `/tmp/zwallet-h18-final-gate-repaired.log`. Only one heavy process suite
ran at a time. Independent review is clear including the header follow-up.
Final-head Linux/macOS CI follows publication; no merge or funded/public send
is claimed by this receipt.
