# v0.4 TUI look — loop

Status: **L1–L3 merged**, 2026-11-10: PR #72 (6787d08), #73 (56d2f0d), #75 (67335b5), with #74 and #77 found by their QA: [`loop.done.md`](loop.done.md) R1–R3. Open: L3.2, the owner's call; follow-ups in #76. Target = [`ssot.md`](ssot.md),
[`../10-tui-look.md`](../10-tui-look.md) units L1–L3 and acceptance L1.1–L3.3. This file holds only
open work; closed rows move to [`loop.done.md`](loop.done.md).

## Round 1 — plan (2026-11-10)

| WU | Repo · branch | Scope | Owner | State |
|---|---|---|---|---|
| — | — | every planned unit merged (`loop.done.md` R1–R3) | — | — |

## Open gaps

| ID | Acceptance | Gap | Source | Status |
|---|---|---|---|---|
| L3.2 | password popup keeps mask, purpose and refusal lines | L3 leaves it unchanged; a modal frame for it waits on the owner (see the L3 adapter table) | owner | 🚪 |

## Spec review

Round 1 (2026-11-10, trinity on 10-tui-look.md): fable-zhuge and astra-zhuge APPROVE-WITH-CHANGES,
grok-elon REJECT. Blocking items adopted into the spec the same day: the widget adapter rule (no
outside value handed to a widget); `FRAME_GLYPHS` cut to the six frame glyphs L1 draws (no marks, no
green, rule 7 untouched); the pinned widget configuration (`KeyGlyphs::Ascii`, `Tabs` without
overflow or drop marker); F2's theme changes written out (no yellow, red or green) with the composed
styles table; `Row::outside` refusing a frame style in every build; L3's field, wrap and toast
constraints; a capture gate per unit. Round 2 reviews the revised spec with L1's diff.

## Next

The password popup's modal frame (L3.2) waits on the owner. Non-blocking follow-ups (frame-style
text typing, frames on Solarized Dark, the ambiguous-wide capture) are in #76.
