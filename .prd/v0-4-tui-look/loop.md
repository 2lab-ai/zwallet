# v0.4 TUI look — loop

Status: **in progress**, 2026-11-10. L1 merged as PR #72 (6787d08): [`loop.done.md`](loop.done.md) R1. Target = [`ssot.md`](ssot.md),
[`../10-tui-look.md`](../10-tui-look.md) units L1–L3 and acceptance L1.1–L3.3. This file holds only
open work; closed rows move to [`loop.done.md`](loop.done.md).

## Round 1 — plan (2026-11-10)

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L2 | zwallet · `feat/tui-look-u2` | the action menu by label (label, key, `[cli spelling]`); 2 wallet's amounts aligned on the decimal point | coding agent + dispatcher gate | in review |
| L3 | zwallet · `feat/tui-look-u3` | review labels grey beside their values (send EVM and Tron, tools); labels type-checked as constants | coding agent + dispatcher gate | in progress |

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L2.1 | the menu shows each action's label; CLI spelling visible and filterable | L2 planned | L2 | 🟡 |
| L2.2 | balances aligned on the decimal point, cut visibly | L2 planned | L2 | 🟡 |
| L2.3 | outside sources, budgets and lines unchanged | L2 planned | L2 | 🟡 |
| L2.4 | real-terminal QA 80x24 and 120x40, captures before and after | L2 planned | L2 | 🟡 |
| L3.1 | reviews keep every field whole; confirm sends exactly the reviewed intent | L3 planned | L3 | 🟡 |
| L3.2 | password popup keeps mask, purpose and refusal lines | L3 leaves it unchanged (no modal frame; see the L3 adapter table) | L3 | 🟡 |
| L3.3 | no outside value handed to a widget; adversarial strings through every adapter | L3 planned | L3 | 🟡 |
| L3.4 | real-terminal QA 80x24 and 120x40, captures before and after | L3 planned | L3 | 🟡 |

## Spec review

Round 1 (2026-11-10, trinity on 10-tui-look.md): fable-zhuge and astra-zhuge APPROVE-WITH-CHANGES,
grok-elon REJECT. Blocking items adopted into the spec the same day: the widget adapter rule (no
outside value handed to a widget); `FRAME_GLYPHS` cut to the six frame glyphs L1 draws (no marks, no
green, rule 7 untouched); the pinned widget configuration (`KeyGlyphs::Ascii`, `Tabs` without
overflow or drop marker); F2's theme changes written out (no yellow, red or green) with the composed
styles table; `Row::outside` refusing a frame style in every build; L3's field, wrap and toast
constraints; a capture gate per unit. Round 2 reviews the revised spec with L1's diff.

## Next

L2 in review (its adapter table passed trinity on 2026-11-10 with changes, written in). L3 starts
from L2's merge, after its adapter table passes trinity.
