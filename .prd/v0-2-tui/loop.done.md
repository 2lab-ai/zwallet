# v0.2 TUI — loop (done)

Append-only ledger of closed WUs and rows moved out of [`loop.md`](loop.md).

## TUI-1a — closed (PR #17)

TUI-1a (the terminal layer and the wallet entry points) merged as PR #17 (08976dc) on 2026-10-28,
after nine review rounds: two reviews each round (an evidence review and a worktree review), the
dispatcher's gate on macOS, x86_64 and Linux, and CI on macOS and ubuntu. Round 8 fixed a Linux-only
failure the gate found (the filter's draw asked a missing terminal for its size); round 9 pinned the
remembered width with two more transitions.

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1a | zwallet · `feat/tui-1-shell` · `.worktrees/feat-tui-1-shell` | ratatui/crossterm, `FilteredBackend` and the terminal guard, `wallet::keys`, `wallet::status`, `wallet::net` with `list` and `status` on them (no acceptance ID) | coding agent + dispatcher gate | merged as PR #17 (08976dc), 2026-10-28, after review rounds 1–9; CI green on macOS and ubuntu at 751ead8. Commits: 66f038c (RED), 4f527f5, 784cfa3, 4f4a153, 5119e36 (c14's clock-step tolerance), 70631ae; review round 1: 95085a8 (RED), e88cd61, e500cad; round 2: b938af1 (RED), a89465d, 9de1521; round 3: d0b1ecf (RED), 0ece36e, 627f367; round 4: 7fcc57b (RED), 99f98dc, 314be53; round 5: 8e45948 (RED), b642bd6, 1d3ec39; 9524890 (the outside daemon's 10m unlock); round 6: 1e68e53 and 40cf954 (RED), df0bd1b, 798a4d8; round 7 (documentation and a test child's own session): 1aadee9, ddc82af; round 8 (Linux gate: the filter's draw no longer asks the terminal for its size): be4d2f0 (RED), 72cb6f0, 58907b4, 500d98d (a test helper's doc comment); round 9 (the reviewers' non-blocking items: a later size narrows the clip and a failed one keeps the last width, mutant M7; wording): 565c21e, then this docs commit. Changed lines (counting rule, before this commit): 1,881 raw (1,743 before round 8), about 1,844 with the 37 lines moved from `list` and `status` into `wallet.rs` counted once; code 820 raw, about 783 (`term.rs` 505 of it: the lines before the test-only `seam` module), tests 954 (`term.rs` 924: the 25-line `seam` module and its test module, the review rounds' panic, guard and filter cases, eight of them child processes; `tests/cli.rs` 30), `.prd` 107. Over the ~800 cap because of the tests the reviews asked for |

## TUI-1b — closed (PR #18)

TUI-1b (`zwallet tui`: the key list, the two-line status bar, the size rule, the cell, style and
budget helpers) merged as PR #18 (24f4b5b) on 2026-10-28, after review round 5 and a green gate
(715cea0). The row and gap below moved out of [`loop.md`](loop.md) on 2026-10-28, when loop.md was
rewritten for v2 (every CLI function).

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1b | zwallet · `feat/tui-1b-screens` · `.worktrees/feat-tui-1b-screens` | `zwallet tui`, the screens, the cell/style/budget helpers, T1 frames, the T9 frames and `q`, the T10 `Msg` test, the cached-address counter-example, README (T1) | coding agent + dispatcher gate | merged as PR #18 (24f4b5b), 2026-10-28, approved after review round 5 (gpt6 APPROVE, code-reviewer APPROVE), gate green on bbe8b5c (macOS, x86, Linux): d7c8323 (RED), fd7e9c9, 75754ae, fd104cc, f8a9fd9, 249c5c7; review round 1: e02db59 (RED), 606bd40, d3668d4; round 2: f5b1640 (RED), 80b8845, 0c248cf; round 3: ab914b3 (RED), 8221618, 15d2020; round 4: f07a184 (RED), 01afa20; round 5: e631c23 (RED), 5cd6512; merges of TUI-1a (d7b91bc, 694eac5, 8ad1d1a, e49f8f1, 6e70102, 841d4b9) and of main after #17 (6299eb5) |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T1 | key list, active marked, lock state in the bar only | closed by `zwallet tui` (TUI-1b, PR #18) | TUI-1b | ✅ |

## Entry condition closed: the explorer re-measure (2026-10-28)

The re-measure that gated the TUI-7+8 PR ran on 2026-10-28, 04:50:25Z–04:50:46Z (the dispatcher;
receipt: the dispatcher's record `explorer-remeasure/record.md` with the raw bodies, in the session scratchpad, outside the repo). The result is identical to 2026-10-27; Routescan's Etherscan-style path
says "chain not supported" for base, arbitrum, polygon and bsc, and only avalanche answers, on both
Routescan paths. Decision, in 03-tui.md §History: no extra parser for base, arbitrum or polygon in
v0.2; avalanche through Routescan stays out of v0.2; the coverage per chain is unchanged.

## R1 — closed 2026-10-30 (v2: TUI-1c … TUI-12, PR #51)

- result: every v2 unit merged in [PR #51](https://github.com/2lab-ai/zwallet/pull/51) (599bde9, merged 2026-10-29T21:34Z = 2026-10-30 KST) and shipped to preview 2026.10.29.2134 · closed rows: T2, T2b, T3, T3b, T4–T17, T19–T44; WUs TUI-1c, TUI-2a, TUI-2b/2c, TUI-3+4, TUI-5 … TUI-12; previews M1–M5 collapsed into that one preview · still open in [`loop.md`](loop.md): T18 (trade-off Q), trade-off B, the owner's acceptance on the preview. (This ledger's first numbered round; the blocks above are named by WU.)

### Closed rows

| WU | Repo · branch · worktree | Scope | Owner | State |
|---|---|---|---|---|
| TUI-1c | zwallet · `feat/tui-1c-pty` · `.worktrees/feat-tui-1c-pty` | the pty runs for T9 and T10, the argv-listed fake, the attacker moved into `tests/common`, the pty helpers (T9, T10) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #31 via #35 |
| TUI-2a | zwallet · `feat/tui-2-warmup` · `.worktrees/feat-tui-2-warmup` | the TUI-2 entry points and the CLI on them, `s` (T2b), A1 ([`units.md`](units.md) §TUI-2 split) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #32 via #35 |
| TUI-2b, TUI-2c | zwallet · branches from TUI-2a | the warm-up, password field and check (T2); pinned addresses on later reads and the open key's trust | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #38 |
| TUI-3+4 | zwallet · at dispatch | "see and receive": `actions.rs` and the form helpers first; tabs, menu, hints; balances, every network, chain switch and session RPC; receive, `V` on every key, `A`, session, `u`, the start flag `--allow-ptrace-scope-0` (T5, T8, T12–T20, T42) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #39; T18's Q3 stays open (trade-off Q) |
| TUI-5 | zwallet · at dispatch | send: payee, amount, review with overrides and fee ceiling, confirm, status, `w` (T6, T6b, T21, T22, T43) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #44 |
| TUI-6 | zwallet · at dispatch | whitelist view and every change, the zero-address refusal at add; pty run across T2, T6, T7 (T7, T11, T23–T25, T44) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #47 |
| TUI-7+8 | zwallet · at dispatch | history (journal, explorer, merge); transaction detail, `dropped/replaced`, `/` lookup (T3, T3b, T4, T6c, T26) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #45 |
| TUI-9 | zwallet · at dispatch | speed up, cancel, retry at nonce N (T27–T29) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #46 (its MUST-FIX fixed in #51, 358b22b) |
| TUI-10 | zwallet · at dispatch | new key, first-run create, `B` as B1 (T30–T32) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #43 |
| TUI-11 | zwallet · at dispatch | call, invoke and its replacements, sign a message, a token by contract (T33–T37) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #48 |
| TUI-12 | zwallet · at dispatch | Tron on every tab (T38–T41) | coding agent + dispatcher gate | merged in PR #51 (599bde9), 2026-10-30, as handoff #49 |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| T2 | warm-up ≈ 1h, open key verified by password, global active key unchanged | merged in PR #51 (599bde9), 2026-10-30; evidence: [tui2-warmup-receipts.md](tui2-warmup-receipts.md) | TUI-2b | ✅ |
| T2b | `s` makes the key active | merged in PR #51 (599bde9), 2026-10-30; evidence: [tui2-warmup-receipts.md](tui2-warmup-receipts.md) | TUI-2a | ✅ |
| T3 | 10 rows, merged, pending first, coverage note | merged in PR #51 (599bde9), 2026-10-30; evidence: [h17-history-receipt.md](h17-history-receipt.md) | TUI-7+8 | ✅ |
| T3b | explorer unavailable: journal only, note says so | merged in PR #51 (599bde9), 2026-10-30; evidence: [h17-history-receipt.md](h17-history-receipt.md) | TUI-7+8 | ✅ |
| T4 | detail fields equal the fixture's, receipt token movements included | merged in PR #51 (599bde9), 2026-10-30; evidence: [h17-history-receipt.md](h17-history-receipt.md) | TUI-7+8 | ✅ |
| T5 | native + non-zero registry tokens | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T6 | send to `alice`, hash at once, mined, journal line | merged in PR #51 (599bde9), 2026-10-30; evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md) | TUI-5 | ✅ |
| T6b | picker: the checked list's address entries on this chain only | merged in PR #51 (599bde9), 2026-10-30; evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md) | TUI-5 | ✅ |
| T6c | `dropped/replaced` | merged in PR #51 (599bde9), 2026-10-30; evidence: [h17-history-receipt.md](h17-history-receipt.md) | TUI-7+8 | ✅ |
| T7 | add `bob` while warm, summary above the password, locked after | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |
| T8 | `c` → base reloads every pane, host redacted | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T9 | 80×24 fits, smaller says so, terminal restored after `q` and a panic | merged in PR #51 (599bde9), 2026-10-30, with TUI-1c (#35): the frames and `q` in TUI-1b, the panic restore in TUI-1a's `term.rs` child tests, the pty run `t9_on_a_pty_q_gives_the_terminal_back_in_canonical_mode_with_echo` (tests/tui.rs) | TUI-1c | ✅ |
| T10 | `zwallet lock` elsewhere shows `locked` within two refreshes (10 s) | merged in PR #51 (599bde9), 2026-10-30, with TUI-1c (#35): the `Msg` test in TUI-1b, the pty run `t10_on_a_pty_zwallet_lock_elsewhere_shows_locked_within_two_refreshes` (tests/tui.rs) | TUI-1c | ✅ |
| T11 | pty across T2, T6, T7: no echo, no password in a frame, cancel and retry | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |
| T12 | the menu's 52 rows equal the Alias column of rows 1–52; each runs or says why not | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T13 | tabs, the hint rule, Esc, the PR's frames at 80×24 | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T14 | unlock for a typed duration | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T15 | every network's balances, all 8 RPCs on mocks | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T16 | the session in full | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T17 | receive: full address and trust | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T19 | `V` verifies every key; `A` repairs the cache | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T20 | the start flag only; `ptrace_scope 0 accepted` shown; no key sets it | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T21 | allow-pending and allow-duplicate on the review | merged in PR #51 (599bde9), 2026-10-30; evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md) | TUI-5 | ✅ |
| T22 | fee ceiling on the review | merged in PR #51 (599bde9), 2026-10-30; evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md) | TUI-5 | ✅ |
| T23 | remove, fee cap, reset | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |
| T24 | contract entry with selectors | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |
| T25 | message signing on or off | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |
| T26 | look a hash up | merged in PR #51 (599bde9), 2026-10-30; evidence: [h17-history-receipt.md](h17-history-receipt.md) | TUI-7+8 | ✅ |
| T27 | speed up at nonce N: pinned address, node-confirmed intent, cap | merged in PR #51 (599bde9), 2026-10-30; evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md) | TUI-9 | ✅ |
| T28 | cancel at nonce N | merged in PR #51 (599bde9), 2026-10-30; evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md) | TUI-9 | ✅ |
| T29 | retry after `BROADCAST_UNCERTAIN`, from this session's review only | merged in PR #51 (599bde9), 2026-10-30; evidence: [replacement receipt](../research/handoff-tui-replacement-2026-10-29.md) | TUI-9 | ✅ |
| T30 | new key, verified, active | merged in PR #51 (599bde9), 2026-10-30; evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md) | TUI-10 | ✅ |
| T31 | first-run create, no phrase or 3-word run in a frame | merged in PR #51 (599bde9), 2026-10-30; evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md) | TUI-10 | ✅ |
| T32 | the phrase: B1's line (B2 only if picked) | merged in PR #51 (599bde9), 2026-10-30, as B1; B2 stays with the owner's trade-off B; evidence: [creation receipt](../research/handoff-tui-create-ui-2026-10-29.md) | TUI-10 | ✅ |
| T33 | `call` and its bounded result | merged in PR #51 (599bde9), 2026-10-30; evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md) | TUI-11 | ✅ |
| T34 | invoke a listed contract and selector; its replacements judged as invoke | merged in PR #51 (599bde9), 2026-10-30; evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md) | TUI-11 | ✅ |
| T35 | invoke with raw calldata | merged in PR #51 (599bde9), 2026-10-30; evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md) | TUI-11 | ✅ |
| T36 | sign a message: the message block, bytes shown = bytes signed | merged in PR #51 (599bde9), 2026-10-30; evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md) | TUI-11 | ✅ |
| T37 | a token by contract, symbol bounded | merged in PR #51 (599bde9), 2026-10-30; evidence: [tools receipt](../research/handoff-tui-tools-2026-10-29.md) | TUI-11 | ✅ |
| T38 | Tron address on 4 receive | merged in PR #51 (599bde9), 2026-10-30; evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md) | TUI-12 | ✅ |
| T39 | TRX and TRC20 USDT balances | merged in PR #51 (599bde9), 2026-10-30; evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md) | TUI-12 | ✅ |
| T40 | send to a listed Tron payee | merged in PR #51 (599bde9), 2026-10-30; evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md) | TUI-12 | ✅ |
| T41 | look a Tron hash up | merged in PR #51 (599bde9), 2026-10-30; evidence: [h18-tron-tui-receipt.md](h18-tron-tui-receipt.md), [h18b](h18b-tron-history-receipt.md) | TUI-12 | ✅ |
| T42 | an RPC URL for this session, host shown, chain id checked | merged in PR #51 (599bde9), 2026-10-30; evidence: [navigation receipt](../research/handoff-tui-navigation-2026-10-29.md) | TUI-3+4 | ✅ |
| T43 | keep waiting for a typed duration | merged in PR #51 (599bde9), 2026-10-30; evidence: [send receipt](../research/handoff-tui-send-2026-10-29.md) | TUI-5 | ✅ |
| T44 | zero address refused at add; an old entry flagged, removable | merged in PR #51 (599bde9), 2026-10-30; evidence: [whitelist receipt](../research/handoff-tui-whitelist-2026-10-29.md) | TUI-6 | ✅ |

### Round section (as it was in loop.md, with the result)

**Result, 2026-10-30.**
- **Merge.** PR #51 (`integ/tui-tron-final`, head 2a1ee64) merged 2026-10-29T21:34Z as 599bde9. It contains #35 and #37–#49, each now marked merged, and supersedes the drafts #31–#33. It closed #34 and #50.
- **Preview.** `preview-2026-10-29-2134-599bde9ee211` (prerelease at 599bde9, 4 binaries + SHA256SUMS) = Homebrew `zwallet-preview 2026.10.29.2134` ([tap PR #18](https://github.com/2lab-ai/homebrew-tap/pull/18), merged 2026-10-29T21:59Z). Installed on the dispatcher's Mac: `brew test` passes, codesign verifies, and the binary's sha256 equals the release asset digest of `zwallet-macos-aarch64`.
- **Gate at 2a1ee64** (the PR head; the merge commit adds no change), Rust 1.95.0: macOS 923 passed, 0 failed, 15 ignored, and every pty-driven test binary again on a pty; Linux Docker (non-root) first full run 915 passed and 1 failed (the pty test `t27_t28_pty_restart_original_proof_speedup_and_cancel_verify_signed_bytes`, tracked as #52), second full run 916 passed (`TEST_EXIT=0`); [CI run 40955576162](https://github.com/2lab-ai/zwallet/actions/runs/40955576162) success on macOS and ubuntu.
- **Real-terminal acceptance** (tmux, release build, loopback mock nodes, sandboxed, no non-loopback connection): all six scenarios PASS at 120x40 and 80x24 — 152 checks on the 2a1ee64 release build, 154 on the installed preview 2026.10.29.2134 (including a tab round trip on the whitelist summary). Coverage matrix ([`../03-tui.md`](../03-tui.md) §Coverage matrix): 59 of 59 rows PASS at each size; 3 rows are not TUI features by design (row 8, a start flag; rows 57 and 58, n/a). Every signed EVM transaction and Tron broadcast equals its review, from the mock nodes' decoded logs. Logs: `~/.local/share/zwallet-epic/logs/qa-terminal/{2a1ee64,2a1ee64-s5rt,preview-599bde9}/` on the dispatcher's machine.
- **Defects found and fixed before the merge**, each RED first (PR #51's body): the #46 policy category taken from the journal (now from the verified bytes); #47's plan fingerprint, Tron payees and summary paging; the real-terminal D1–D4; the whitelist list wrapping; the explorer override (`ZWALLET_EXPLORER_<CHAIN>`); the tab-round-trip losses (whitelist plan, tools reviews); the guard labels.

**Start condition.** Met: TUI-1a (PR #17) and TUI-1b (PR #18, 24f4b5b) are merged. G3 and G4 of [`../v0-1/loop.md`](../v0-1/loop.md) wait only on owner actions and do not block TUI code.

**Entry conditions.**
- Satisfied 2026-10-28: the explorer re-measure that gated TUI-7+8 (the block "Entry condition closed" above).
- Trade-offs Q and B (03-tui.md §Open trade-offs) go to the owner. TUI-3+4 and TUI-10 built the CLI pointers (Q1, B1); Q3 lands as a delta of T18 if picked; B2 is built only if the owner picks it (recommended: B1). Still open: [`loop.md`](loop.md).
- TUI-12 starts after 05-tron.md (branch `docs/tron-spec`) and its library units. Met: TUI-12 merged as #49.

**PRs.** One risk boundary per PR ([`units.md`](units.md) §PRs and previews, the dispatcher's plan after review round 1 of v2, 2026-10-28). Rows: Closed rows above.

**Previews.** M1 (TUI-2 and TUI-3+4), M2 (TUI-5 and TUI-6), M3 (TUI-7+8 and TUI-9), M4 (TUI-10 and TUI-11), M5 (TUI-12), each planned; collapsed into one preview of the PR #51 merge, 2026.10.29.2134.

**Build facts.**
- Gate: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test
  --locked`, the cli and tui suites again on a pty ×3 (`script -q /dev/null cargo test --locked
  --test cli < /dev/null`, the same with `--test tui`), live `cargo test --locked -- --ignored`,
  and Linux/macOS CI on the pinned Rust 1.95.0 toolchain.
- Tests start debug binaries with an empty environment and the `/tmp/zwt-` daemon scope;
  production sessions are outside that scope. `lock`, whitelist edits and `L` tests hold
  `common::exclusive()`. Never stop a real session to run tests. Run one full CLI suite at a
  time; the cross-process fixture lock does not make concurrent heavy suites efficient.
- Acceptance frames: tmux at 120×40 and 80×24, `capture-pane`, read by the dispatcher
  (03-tui.md §Acceptance); T3 and T3b on in-test `TestBackend` frames. Every `TestBackend` frame
  goes through `FilteredBackend` with zero replaced cells. No frame of a real wallet's phrase.

**Open gaps (intro).** As of 2026-10-30 every unit was implemented on `integ/tui-tron-final`, which merged the handoff PRs #35 and #37–#49 with main; each row stayed open until that PR merged. Only T18's owner trade-off remains open after that.

**From "Open for TUI-2b".** Round 4 of TUI-1c's review closed a third line, the stderr join in `PtyChild::stderr()`: a daemon the warm-up starts inherits neither that pipe nor the pty slave, since `spawn_daemon` gives it `Stdio::null()` for stderr and marks every fd from 3 up close-on-exec (src/core/protocol.rs:1354, :1363). The other two lines stay open in [`loop.md`](loop.md).

**Next (as it was).** Merge `integ/tui-tron-final` as one PR (it marks #35 and #37–#49 merged), then move the 🟡 rows to `loop.done.md`. The previews M1–M5 can collapse into one preview of that merge: the owner unlocks again (new cdhash) and accepts the TUI on it. T18's trade-off Q stays with the owner; the existing Q1 and B1 pointers remain the defaults, and the handoff does not authorize phrase display or a new QR implementation. Done 2026-10-30 except the owner's steps.
