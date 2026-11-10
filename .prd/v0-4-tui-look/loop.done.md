# v0.4 TUI look — loop (done)

Append-only ledger of closed WUs and rows moved out of [`loop.md`](loop.md).

## R1 — L1 closed (PR #72)

L1 (the chrome: `twolab-tui` pinned at 2e224c8, `style::THEME` and the frame styles, the write
point's frame exception, rounded pane frames, the `Tabs` tab line, the `KeyHints` hint rows, the
bar's colours, the two v0.2 test notes) merged as PR #72 (6787d08) on 2026-11-10, after trinity
rounds 1–3 on the spec and the code (round 3: astra-zhuge, grok-elon and fable-zhuge
APPROVE-WITH-CHANGES, no P0/P1, the changes in a3b376f) and a `zworkflow:code-reviewer` pass (its
P0, a test call missing an argument, fixed in a3b376f). CI green on macOS and ubuntu at a3b376f.

### Closed rows

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L1 | zwallet · `feat/tui-look-u1` | `twolab-tui` pinned at 2e224c8; `style::THEME`, `FRAME`, `FRAME_GLYPHS`; the write point's second exception; rounded pane frames; tab line (`Tabs`); hint row (`KeyHints`); bar colours; the two v0.2 test notes | coding agent + dispatcher gate | merged as PR #72 (6787d08), 2026-11-10: d331bf1 (spec), a2649d6, 18ebb32 (trinity round 2), a3b376f (trinity round 3) |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L1.1 | every TUI test frame passes amended rules 1 and 2, zero replacements | closed: every frame helper accepts a frame glyph only in a frame style and counts replacements; CI green at a3b376f | L1 | ✅ |
| L1.2 | frame glyph outside a frame style, and any other non-ASCII glyph, is `?` | closed: `only_the_frame_styles_let_the_six_frame_glyphs_through` (`src/tui/term.rs`) | L1 | ✅ |
| L1.3 | `Row::outside` refuses a frame style; no placed outside value in a frame style | closed: `outside_style` in every build, its release-mode test, the debug panic test; `style::frame_style_in` asserted for every placed value in all nine frame helpers | L1 | ✅ |
| L1.4 | frame styles disjoint from the text styles; equal to the widgets' styles under `THEME` | closed: `the_frame_styles_are_the_widgets_and_none_is_a_text_style` and the adapter tests in `src/tui/ui.rs` | L1 | ✅ |
| L1.5 | screen text unchanged except frame glyphs and tab spacing | closed: the frame checks read screens through `style::ascii_chrome` and every existing layout assertion passes; hint rows keep their words (`every_hint_row_shows_all_its_hints_at_80`; one row too wide for two spaces falls back to one) | L1 | ✅ |
| L1.6 | v0.2 test notes: grow back after shrink; bytes after the first `CSI ?1049l` checked | closed: `t9` grows the window back; `quit()` compares the bytes after the first `CSI ?1049l` with exactly `CSI ?25h`, after the pty reader ended | L1 | ✅ |
| L1.7 | real-terminal QA 80x24 and 120x40 on L1's build, captures before and after | closed: the dispatcher's tmux harness on a scoped debug build of a3b376f (opt-level 3), s1–s7 at both sizes: 176/176 checks, 59/59 reachable coverage rows at both sizes; owner's daemon untouched, no QA daemon left, no traffic off loopback; before/after captures in the task thread. Not done: the ambiguous-wide capture (no terminal set that way here) | L1 | ✅ |
