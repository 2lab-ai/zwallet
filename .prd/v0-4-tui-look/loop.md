# v0.4 TUI look — loop

Status: **in progress**, 2026-11-10. Target = [`ssot.md`](ssot.md),
[`../10-tui-look.md`](../10-tui-look.md) units L1–L3 and acceptance L1.1–L3.3. This file holds only
open work; closed rows move to [`loop.done.md`](loop.done.md).

## Round 1 — plan (2026-11-10)

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L1 | zwallet · `feat/tui-look-u1` | `twolab-tui` pinned at 2e224c8; `style::THEME`, `FRAME`, `FRAME_GLYPHS`; the write point's second exception; rounded pane frames; tab line (`Tabs`); hint row (`KeyHints`); bar colours; the two v0.2 test notes | coding agent + dispatcher gate | in progress |
| L2 | zwallet · `feat/tui-look-u2` | 2 wallet ledger, every-network columns, the action menu by label and tab | coding agent + dispatcher gate | planned, after L1 |
| L3 | zwallet · `feat/tui-look-u3` | approval-card reviews, modal password popup, toast outcomes | coding agent + dispatcher gate | planned, after L1 |

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L1.1 | every TUI test frame passes amended rules 1 and 2, zero replacements | L1 not merged | L1 | 🔨 |
| L1.2 | frame glyph outside a frame style, and any other non-ASCII glyph, is `?` | L1 not merged | L1 | 🔨 |
| L1.3 | `Row::outside` refuses a frame style; no placed outside value in a frame style | L1 not merged | L1 | 🔨 |
| L1.4 | frame styles disjoint from the text styles; equal to the widgets' styles under `THEME` | L1 not merged | L1 | 🔨 |
| L1.5 | screen text unchanged except frame glyphs and tab spacing | L1 not merged | L1 | 🔨 |
| L1.6 | v0.2 test notes: grow back after shrink; bytes after the first `CSI ?1049l` checked | L1 not merged | L1 | 🔨 |
| L1.7 | real-terminal QA 80x24 and 120x40 on L1's build, captures before and after, one ambiguous-wide | L1 not merged | L1 | 🔨 |
| L2.1 | the menu shows each action's label; CLI spelling visible and filterable | L2 planned | L2 | 📋 |
| L2.2 | balances aligned on the decimal point, cut visibly | L2 planned | L2 | 📋 |
| L2.3 | outside sources, budgets and lines unchanged | L2 planned | L2 | 📋 |
| L2.4 | real-terminal QA 80x24 and 120x40, captures before and after | L2 planned | L2 | 📋 |
| L3.1 | reviews keep every field whole; confirm sends exactly the reviewed intent | L3 planned | L3 | 📋 |
| L3.2 | password popup keeps mask, purpose and refusal lines | L3 planned | L3 | 📋 |
| L3.3 | no outside value handed to a widget; adversarial strings through every adapter | L3 planned | L3 | 📋 |
| L3.4 | real-terminal QA 80x24 and 120x40, captures before and after | L3 planned | L3 | 📋 |

## Spec review

Round 1 (2026-11-10, trinity on 10-tui-look.md): fable-zhuge and astra-zhuge APPROVE-WITH-CHANGES,
grok-elon REJECT. Blocking items adopted into the spec the same day: the widget adapter rule (no
outside value handed to a widget); `FRAME_GLYPHS` cut to the six frame glyphs L1 draws (no marks, no
green, rule 7 untouched); the pinned widget configuration (`KeyGlyphs::Ascii`, `Tabs` without
overflow or drop marker); F2's theme changes written out (no yellow, red or green) with the composed
styles table; `Row::outside` refusing a frame style in every build; L3's field, wrap and toast
constraints; a capture gate per unit. Round 2 reviews the revised spec with L1's diff.

## Next

L1 first: it changes the chrome every pane shares. L2 (the menu first, then the ledger) and L3
start from L1's merge, each after its adapter table passes trinity.
