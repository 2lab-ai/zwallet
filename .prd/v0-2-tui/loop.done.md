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
