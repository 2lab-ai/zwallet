# v0.4 TUI look — loop

Status: **in progress**, 2026-11-10. L1 merged as PR #72 (6787d08), L2 as PR #73 (56d2f0d): [`loop.done.md`](loop.done.md) R1, R2. Target = [`ssot.md`](ssot.md),
[`../10-tui-look.md`](../10-tui-look.md) units L1–L3 and acceptance L1.1–L3.3. This file holds only
open work; closed rows move to [`loop.done.md`](loop.done.md).

## Round 1 — plan (2026-11-10)

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| L3 | zwallet · `feat/tui-look-u3` | review labels grey beside their values (send EVM and Tron, tools); labels type-checked as constants | coding agent + dispatcher gate | in review |

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L3.1 | reviews keep every field whole; confirm sends exactly the reviewed intent | L3 planned | L3 | 🟡 |
| L3.2 | password popup keeps mask, purpose and refusal lines | L3 leaves it unchanged; a modal frame for it waits on the owner (see the L3 adapter table) | owner | 🚪 |
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

L3 in review (its adapter table passed trinity on 2026-11-10: two APPROVE, one APPROVE-WITH-CHANGES,
the changes written in). The password popup's modal frame (L3.2) waits on the owner.
