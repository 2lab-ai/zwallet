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

## R2 — L2 closed (PR #73)

L2 (the `?` action menu named by what each action does, then its key, then its CLI spelling; 2
wallet's amounts aligned on the decimal point) merged as PR #73 (56d2f0d) on 2026-11-10, after
trinity on the L2 adapter table and the code (astra-zhuge, grok-elon and fable-zhuge
APPROVE-WITH-CHANGES; their P1s, three CLI spellings cut at 80 columns and one huge amount cutting
the others, fixed in 31d8169). CI green on macOS and ubuntu at aecce04.

### Closed rows

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L2 | zwallet · `feat/tui-look-u2` | the action menu by label (label, key, `[cli spelling]`); 2 wallet's amounts aligned on the decimal point | coding agent + dispatcher gate | merged as PR #73 (56d2f0d), 2026-11-10: 9996fd4, 31d8169 (trinity), 116ef11, aecce04 |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L2.1 | the menu shows each action's label; CLI spelling visible and filterable | closed: `the_menu_columns_keep_every_label_and_cli_spelling_whole_at_80` over the whole action table; the filter matches label or CLI spelling as before | L2 | ✅ |
| L2.2 | balances aligned on the decimal point, cut visibly | closed: `nav_balance_amounts_line_up_on_the_decimal_point`, `nav_a_huge_amount_never_cuts_a_small_one` | L2 | ✅ |
| L2.3 | outside sources, budgets and lines unchanged | closed: the CLI spelling and the typed filter stay `Outside::input`; every placement check passes | L2 | ✅ |
| L2.4 | real-terminal QA 80x24 and 120x40, captures before and after | closed: the tmux harness on a scoped debug build of aecce04 (opt-level 3), s1–s7 at both sizes: 175/176 checks (s5–s7 run again after a session restart, 88/88); owner's daemon untouched, no QA daemon left, no traffic off loopback; before/after captures in the task thread. The one failure, `s2 80x24 Q`, is not L2's: the `qrcode` crate made that run's address a version 4 code, too big for 80x24; fixed by PR #74 (byte mode) | L2 | ✅ |

## R3 — L3 closed (PR #75), two fixes found by its QA (PR #74, PR #77)

L3 (review labels dim beside their values on the send reviews, EVM and Tron, and the tool rows;
labels typed `&'static str`) merged as PR #75 (67335b5) on 2026-11-10. Trinity on the L3 adapter
table and code: astra-zhuge and grok-elon APPROVE, fable-zhuge APPROVE-WITH-CHANGES, no P0/P1. The
label colour then changed twice on evidence: the first capture showed `SECONDARY` (Gray) drew no
visible change on a dark default and almost vanished on a light terminal (851e4bc moved to
DarkGray); fable-zhuge on 851e4bc found DarkGray is Solarized Dark's background, so `To:`/`From:`
would vanish (97a65c3 moved to `HINT`, dim). CI green on macOS and ubuntu at 97a65c3.

Found by the QA of L2 and L3 and fixed in their own PRs:
- PR #74 (4b797ea): the `qrcode` crate's mixed-mode encoder made about one EIP-55 address in three a
  version 4 code (41 columns), so 4 receive showed no QR at 80x24; byte mode makes every address
  version 3. `zworkflow:code-reviewer` APPROVE (20,000 random addresses: 31% before, 0% after).
- PR #77 (9884018): L1's hint row drew what a key does in Gray (`#bfbfbf` on a white Terminal.app) and its
  notes in DarkGray; it now redraws both in `HINT`, as before L1. `zworkflow:code-reviewer` APPROVE.

### Closed rows

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L3 | zwallet · `feat/tui-look-u3` | review labels dim beside their values (send EVM and Tron, tools); labels type-checked as constants | coding agent + dispatcher gate | merged as PR #75 (67335b5), 2026-11-10: ce8ba00, dd591b0 (trinity), 4391178 (L2 ledger), 851e4bc, 97a65c3 (fable-zhuge) |

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L3.1 | reviews keep every field whole; confirm sends exactly the reviewed intent | closed: no text, row or key changed; `tests/transfer_review.rs` 25/25 and `tests/tron_tui.rs` 21/21; QA s3 and s6 check one signed tx equal to the review (mock log) | L3 | ✅ |
| L3.3 | no outside value handed to a widget; adversarial strings through every adapter | closed: labels reach `label_style` only as `&'static str`; values stay `Row::outside` in their old style; `tui_review_labels_are_dim_and_values_keep_their_style`, `l3_tron_review_labels_are_dim_and_values_keep_their_style` | L3 | ✅ |
| L3.4 | real-terminal QA 80x24 and 120x40, captures before and after | closed: the tmux harness on eaf25d2 (L3 + #74) and on 3afd636 (L3 + #74 + #77, whose tree main 9884018 has byte for byte), s1–s7 at both sizes: 176/176 each; owner's daemon untouched, no QA daemon left, no traffic off loopback; before/after of the send review and the hint row rendered in three palettes (iTerm2 dark, Terminal.app light, Solarized Dark) in the task thread | L3 | ✅ |
