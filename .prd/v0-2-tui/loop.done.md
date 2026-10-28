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
