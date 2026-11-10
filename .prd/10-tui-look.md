# zwallet — TUI look (v0.4)

Status: **in progress**, 2026-11-10. Target: [`v0-4-tui-look/ssot.md`](v0-4-tui-look/ssot.md).
Driver: [`v0-4-tui-look/loop.md`](v0-4-tui-look/loop.md). It changes how the TUI of
[`03-tui.md`](03-tui.md) looks, not what it does: every function, key, outside source and budget of
03-tui.md stays as it is.

## Problem

The owner, 2026-11-10: "tui 개 좇같으니까 2lab.ai/tui 이용해서 개선해줘". The installed preview
(2026.11.06.2001, 2eab208), as the real-terminal QA captured it at 120x40
(`s2-120x40-02-action-menu`, `s2-120x40-04-warmed-up`, `s2-120x40-06-balances-every-network`):

- every pane is an ASCII box (`+`, `-`, `|`) around a flat list of grey or default text;
- colour exists only for a warning (yellow) and an error (red);
- the tab line is eight words with the current one reversed;
- the hint row is one run of dim words: `Tab focus  j/k  Enter detail  / hash  a networks ...`; keys
  and what they do look the same;
- the status bar is two plain lines joined by ` | `; "verified", "not verified" and "signing" look
  alike;
- the action menu lists CLI spellings (`[transfer --allow-pending]`), not what an action does;
- balances are unaligned prose (`1234.5 USDC`, `base: 1.5 ETH, 1234.5 USDC, 0.25 WETH`).

## Goal

The same TUI in the look of `twolab-tui` (<https://2lab.ai/tui>, `github.com/2lab-ai/tui`, ratatui
0.30, Apache-2.0, 2lab's own widget library): rounded frames in a quiet colour, one accent colour,
the current tab plainly marked, keys bold in the hint row, status words coloured by what they mean,
numbers aligned on their decimal point, actions named by what they do.

Non-goals: no new function, no key change, no change to which outside values are shown, where, or
within which budget (03-tui.md §Display integrity rules 3 and 4); no motion that needs a clock.

## Display integrity: what changes (03-tui.md §Display integrity rules 1 and 2)

**Rule 1 today.** A cell holds one printable ASCII character or `₮`; the one exception is the QR
code's half blocks (`█ ▀ ▄`) in the `QR` style (trade-off Q, Q5). The chrome keeps to ASCII
(`+ - |`, `>`, `*`, `...`).

**Amendment F1: frame glyphs in frame styles.** A second exception: the six glyphs of
`style::FRAME_GLYPHS`, the rounded light frame `╭ ╮ ╰ ╯ ─ │`, pass the write point
(`FilteredBackend`, `src/tui/term.rs`) only in a cell whose style is one of `style::FRAME`, the
chrome styles. Nothing else is added: no mark (`✓ ✗ ▸ •`), no `·`, no `…`, no arrow, no block
element. Every other non-ASCII glyph in any style, and a frame glyph in any other style, is still
`?` and counted. A unit that wants another glyph amends this list first, with its reason.

Why it keeps the threat out (03-tui.md §Display integrity, first paragraph: a process of this user
controls some text zwallet shows and must never make zwallet's screen show an address as verified
or fundable):

1. **No outside value is ever drawn in a frame style.** `Row::outside` is the only way an outside
   value reaches a cell, and it refuses a frame style in every build: the value is drawn in
   `NORMAL` instead (and a debug build panics, so a test finds the call). Every frame the tests draw
   asserts that the cells of every placed outside value hold a non-frame style. So a frame glyph
   that reaches the terminal is zwallet's own.
2. **The widget adapter rule.** A `twolab-tui` widget is handed only zwallet's own text: labels,
   key names, words that are constants in the code. No `Outside` value, and no string built from
   one, is ever passed to a widget; outside values go through `Row` with their source, budget,
   placement record and a non-frame style, as today. L1 uses two widgets, `Tabs` (the seven tab
   names) and `KeyHints` (the hint row's constant words), and both are built from `&'static str`
   only. A test feeds every adapter its widest constant and asserts zero replacements.
3. Outside text is already printable ASCII when it reaches `Row` (rule 3, `escape`); the write
   point stays the last filter, as `write_human` is the CLI's.
4. Rule 5 is unchanged: none of the frame glyphs is a block element, the half blocks still pass only
   in `QR`, and no frame style equals `QR` (white on black).
5. Rule 7 is unchanged: no frame style is green, no mark is added, and colour adds no word. The
   trust words keep the styles they have today (`NORMAL` for "verified by ...", `HINT` for "not
   verified", `WARNING` for an impostor).

**Widget configuration, pinned.** `KeyHints` uses `KeyGlyphs::Ascii` (arrows spelled out, the
overflow marker `...`) and is laid out at the row's width; every hint row is zwallet's own and fits,
and a lone hint wider than the row would end in `…`, which the write point replaces (fails closed). `Tabs` uses `Hints::Off`, the divider ` │ `, no scroll buttons,
no drop marker and no action; its seven labels take 75 of the 80 columns, so it never overflows (a
test draws it at 80 columns and asserts every label is whole and no `‹ ›` is drawn).

**Amendment F2: the frame styles are constants too (rule 2).** `src/tui/style.rs` keeps every style
the TUI draws as a constant. `style::THEME` is zwallet's own `Theme` value: the 16 named colours of
`Theme::ansi()` (the user's palette decides the hues, so light and dark terminals both work) with
four changes, because `Theme::ansi()` itself uses yellow and red for `wait`, `warn`, `fail` and
`crit`, which equal `WARNING` and `ERROR`: `wait` and `warn` are `LightYellow`, `fail` and `crit`
are `LightRed`, and `done` and `ok` are `Reset` (no green anywhere in zwallet's theme). `FRAME`
lists exactly the styles the L1 chrome and widgets draw, composed as ratatui composes them:

| Constant | Style | Drawn by |
|---|---|---|
| `BORDER` | fg `DarkGray` | pane frames (`border_style`), the bar's ` \| ` separators, `Tabs`' divider (`THEME.dim()`) |
| `ACCENT` | fg `Cyan` | `Tabs`' other labels (`THEME.accent()`), a pane's title |
| `ACCENT_BOLD` | fg `Cyan`, bold | `KeyHints`' keys (`accent()` + bold), the bar's chain |
| `TAB` | reversed, fg `Cyan`, bold | `Tabs`' current label (`selected()` patched with `accent()`, + bold); no underline with `Hints::Off` |
| `SECONDARY` | fg `Gray` | `KeyHints`' labels (`THEME.secondary()`) |

A unit test asserts each constant equals the style its widget composes from `THEME` (the theme
methods, composed as the widget composes them) and that no frame style equals `NORMAL`, `TITLE`,
`SELECTED`, `HINT`, `WARNING`, `ERROR` or `QR`; the adapter tests render `Tabs` and `KeyHints` into a
buffer and check every cell's glyph and style. `style::ALL` is the old seven plus `FRAME`, so every frame test's rule 2
check covers them. Under `NO_COLOR` crossterm writes no colour; bold, reverse and the glyphs keep
the meaning. Rule 2's sentence "each cell carries exactly one of them" stays true of the result: a
widget's patched style is one of the constants above, which the cell check proves.

**Rule 1's chrome sentence** becomes: borders are the rounded light frame in `BORDER`; the tab
divider is ` │ ` in `BORDER`; separators in the hint row and the bar stay ASCII spaces and `|`; the
selection marker stays `>`; the password mask stays `*`; busy states still end in `...`.

**Known limit.** All six frame glyphs are East-Asian Ambiguous (Unicode `EastAsianWidth.txt`: `A`
for U+2500, U+2502 and U+256D–U+2570): a terminal set to draw ambiguous characters wide (some CJK
setups) draws them two columns wide, as it does every ratatui application's borders. Every ASCII
character of the screen stays narrow. Not handled in v0.4; L1's QA records one capture with
ambiguous-wide on.

## Units

| Unit | What the owner sees once it is merged | Uses from twolab-tui | Acceptance |
|---|---|---|---|
| L1 chrome | rounded frames on every pane; the tab line with the current tab marked; the hint row with keys bold cyan and their actions grey; the chain and the bar's separators coloured | `Theme`, `Tabs`, `KeyHints` | L1.1–L1.7 |
| L2 content | the action menu names what an action does, the CLI spelling dim beside it, grouped by tab; 2 wallet's balances as a ledger aligned on the decimal point, every network as columns | `ledger` alignment math, `rule` (zwallet's own labels only) | L2.1–L2.4 |
| L3 reviews and popups | send, whitelist and tool reviews laid out as cards; the password popup in a modal frame | frames and layout only; every field still drawn by `Row` | L3.1–L3.4 |

**Before L2 and L3 start**, each amends this spec with its widget adapter table (widget, what zwallet
hands it, the glyphs and styles it draws, the F1 list or `FRAME` entries it adds) and gets a trinity
pass on that table. Constraints already fixed for L3: a review never drops or cuts a field it shows
today (the full payee address, the exact message bytes of the message block, the guards, the fee
ceiling); its confirm keys stay; a modal body wraps only where rule 4 already lets that source wrap;
no toast with a timer (a toast-like outcome row stays until the next key, and holds no outside
value in a frame style). No mark that could read as "verified" (`✓`, green) is drawn next to an
address or a trust word.

### L2 adapter table (2026-11-10)

| Part | Widget | What zwallet hands it | Glyphs and styles | F1 / `FRAME` change |
|---|---|---|---|---|
| the action menu | none: every row is a `Row` | each action's key, label and CLI spelling are constants of the action table (`src/tui/actions.rs`); the filter is the human's typed input (`Outside::input`, as today) | ASCII only; key in `ACCENT_BOLD` on an unselected row, label in `NORMAL`, the CLI spelling in `HINT`; the selected row all in `SELECTED`; the tab of the selected action named on the detail line | none |
| 2 wallet's balances | none: `Row`, padded so the decimal points line up (the idea of `twolab_tui::ledger`, not the widget: `ledger::Amount` takes an `f64` and a currency sign, which would lose a token amount's digits) | amounts are `Outside` (`Source::Quantity`), drawn by `Row` | ASCII only; the symbol column in `NORMAL` | none |

The menu keeps its order, its filter (CLI spelling or label) and its keys: only what each row says
changes, from `NN key [cli spelling] tab N` to `key  what it does  cli spelling`.

## Acceptance

- **L1.1** Every frame of every existing TUI test passes the amended rule 1 and rule 2 checks with
  zero replacements by the write point.
- **L1.2** The write point turns a frame glyph into `?` in any non-frame style, and every other
  non-ASCII glyph into `?` in any style, frame styles included (`src/tui/term.rs` tests).
- **L1.3** `Row::outside` with a frame style panics; the frame checks assert no placed outside value
  sits in a frame style.
- **L1.4** No frame style equals `NORMAL`, `TITLE`, `SELECTED`, `HINT`, `WARNING`, `ERROR` or `QR`;
  each frame style equals what its `twolab-tui` widget draws with `style::THEME`; `THEME` has no
  `Yellow`, `Red` or `Green`.
- **L1.5** The text on every screen is what it was, except the frame glyphs and the tab line's
  spacing (tests that read the old `+`/`|` frame are updated, nothing else).
- **L1.6** The two open test notes of v0.2 (loop.md §Open test notes): a pty run grows the window
  back after shrinking it; the bytes after the first `CSI ?1049l` are checked too.
- **L1.7** Real-terminal QA (the dispatcher's tmux harness) at 80x24 and 120x40 passes every
  scenario on L1's build, with before and after captures in the PR; one capture with
  ambiguous-wide on.
- **L2.1** Every action in the menu shows its label; its CLI spelling stays visible and filterable.
- **L2.2** Balances line up on the decimal point; a value that does not fit is cut with `...`, never
  silently.
- **L2.3** Outside values keep their sources, budgets and lines (rule 3 and 4 tests unchanged).
- **L2.4** Real-terminal QA at 80x24 and 120x40 with before and after captures.
- **L3.1** A review shows every field it shows today, whole, in the card; confirm still sends
  exactly the reviewed intent.
- **L3.2** The password popup keeps the mask, the purpose line and the refusal line.
- **L3.3** No outside value is handed to a widget; adversarial strings through every adapter give
  zero replacements and no frame style on an outside cell.
- **L3.4** Real-terminal QA at 80x24 and 120x40 passes every scenario, with captures before and
  after.
