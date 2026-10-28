# Research — keyboard TUIs and wallet UX, for `zwallet tui` v2 (as of 2026-10-28)

Status: **reference**, 2026-10-28. Input to [`../03-tui.md`](../03-tui.md) §Navigation and
§Coverage matrix, not a contract. Every web source was opened on 2026-10-28 unless marked
otherwise. `[unverified]` marks a claim no opened source states. ratatui facts come from the
crate sources this repo pins (Cargo.lock: ratatui 0.30.2, ratatui-core 0.1.2, ratatui-widgets
0.3.2), read in the cargo registry.

Why now: on 2026-10-28 the owner saw a TUI that shows only the key list (the dispatcher's tmux
frames, `scratchpad/qa-tui/1-locked-120x40.txt` to `9-real-unlocked.txt`: hint line `j/k move
r refresh L lock every zwallet daemon q quit`, nothing else) and asked that every feature be
usable from it.

## Bottom line

1. **One key lists every action and runs it.** lazygit (`?` opens a menu, `/` filters, Enter
   executes), yazi (F1 or `~` opens help, Enter "Close help menu and run selected action(s)") and
   k9s (`?` shows the key mnemonics, `:` opens a view by name or alias) all give a first-time
   user one place where every function is listed. zwallet takes one **action menu** on `?` and
   `:`: filterable, Enter runs, each row with its key and the CLI command it matches.
2. **Number keys switch the top-level views.** gitui `1`–`5` tabs, yazi `1`–`9` tabs, lazygit
   `1`–`5` panels, btop `1`–`4` boxes; lazygit and yazi use `[` and `]` for the previous and next
   tab. zwallet takes `1`–`7` and `[` `]`.
3. **Esc backs out; `q` and Ctrl-C quit.** The same in k9s, lazygit and yazi. zwallet quits on
   `q` and Ctrl-C today (src/tui/app.rs:205); it reads Esc as a key (src/tui/mod.rs:148) but
   gives it no action yet.
4. **A hint line names the keys of the current view.** lazygit's `showBottomLine` is on by
   default; gitui has a command bar that `.` expands. zwallet keeps its hint line (src/tui/ui.rs:94-100)
   and makes it per screen, ending in `? all actions`.
5. **Wallets gate the receive QR on a verified address.** Ledger Live shows "a receive address
   and QR code" after device verification; an address made without the device "does not have the
   added security protection". zwallet's gate is the same: `verified by password` or
   `verified by session` (03-tui.md §Address trust), as the CLI's `with_qr` (commands.rs:620).
6. **Send is recipient → amount → fee → summary → confirm on a trusted surface.** Ledger Live
   orders it so and confirms on the device; Sparrow splits create, finalize, sign and broadcast.
   zwallet keeps payee → amount → review → type the nickname → status (03-tui.md T6).
7. **An address book as a gate.** Rabby added a "Whitelist" against look-alike addresses and
   warns "This address is not whitelisted" before signing (its own posts on X; the text was read
   in a search result, the posts were not opened: `[unverified]`). zwallet's list is stronger:
   the key daemon refuses an unlisted payee (README.md:284-301).
8. **Stuck sends: speed up and cancel at the same nonce.** MetaMask re-sends at the same nonce
   with a higher fee, and cancels with a transaction to your own address at that nonce ("does not
   actually have to have any value"); it asks at least +10% priority fee and +30% max fee.
   zwallet has the parts: `--nonce N --replace <hash> --bump <factor>` (default 1.25,
   README.md:594-622).
9. **Readable summaries before signing.** Frame turns calldata into summaries with verified
   ABIs (its repository is archived). zwallet's equivalent is its own decoder of the unsigned
   transaction (`tx::decode_unsigned`, chains/evm/tx.rs), which the invoke review shows.
10. **ratatui defaults break zwallet's cell rule.** Block borders, the Tabs divider, the
    Scrollbar, Gauge and LineGauge draw box-drawing or block glyphs by default; each is either
    given an ASCII set or not used (§ratatui below).

## Keyboard-first TUIs

| Reference | What it does | Source | Taken for zwallet |
|---|---|---|---|
| lazygit | `?` "Open keybindings menu"; the menu: Enter executes, Esc closes, `/` filters; confirmation panel: Enter confirms, Esc cancels; `q`, Ctrl-C quit | [Keybindings_en.md](https://github.com/jesseduffield/lazygit/blob/master/docs/keybindings/Keybindings_en.md) | the action menu (`?`), confirms on Enter/Esc |
| lazygit | defaults `optionMenu: '?'`, `jumpToBlock: 1..5`, `nextTab: ']'`, `prevTab: '['`; `showBottomLine: true` ("the bottom line that contains keybinding info"); `showPanelJumps` shows the jump keys in window titles | [Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md) | number keys in the tab line, `[` `]`, a hint line |
| lazygit | undo with `z` ("uses the reflog") | [README](https://github.com/jesseduffield/lazygit) | not taken: a broadcast transaction cannot be undone; speed up and cancel are its nearest relatives |
| k9s | `?` "Show active keyboard mnemonics and help"; `:` "View a Kubernetes resource using singular/plural or short-name"; `/` filter; Esc "go up/back to the previous view"; Ctrl-A "Show all available resource alias"; `:q` quits | [README](https://github.com/derailed/k9s), [commands](https://k9scli.io/topics/commands/) | `:` opens the same menu; each action's alias is its CLI command name, so `:whitelist add` finds it |
| k9s | custom hotkeys "will be listed in the help view `?`" | [hotkeys](https://k9scli.io/topics/hotkeys/) | every key, screen-local or global, is in the one menu |
| gitui | defaults: `tab_status` `1` … `tab_stashes` `5`, Tab/Shift-Tab cycle, `cmd_bar_toggle` `.`, `open_help` `h`, `exit_popup` Esc | [key_list.rs](https://raw.githubusercontent.com/gitui-org/gitui/master/src/keys/key_list.rs) | tabs on digits; a one-line hint (no expand key: the menu covers it) |
| gitui | "Context based help (no need to memorize tons of hot-keys)" | [README](https://github.com/gitui-org/gitui) | hints change with the screen |
| yazi | "F1 or ~ to open the help menu"; help: Enter "Close help menu and run selected action(s)"; `1`–`9` tabs, `[` `]` previous/next | [quick start](https://yazi-rs.github.io/docs/quick-start), [keymap-default.toml](https://raw.githubusercontent.com/sxyazi/yazi/main/yazi-config/preset/keymap-default.toml) | help and command palette are one list that runs what is picked |
| btop | help rows: "Esc, m" main menu; "F2, o" options; "F1, ?, h" help; `1`–`4` toggle boxes; "f, /" filter; `t`, `k`, `s` send signals to the selected process | [btop_menu.cpp](https://raw.githubusercontent.com/aristocratos/btop/main/src/btop_menu.cpp) | `?` as help; a destructive key on a selected row (`L` locks; `R` resets with the password) |
| btop | "all buttons with a highlighted key are clickable" | [README](https://github.com/aristocratos/btop) | not taken: keyboard only, one input path, as the pty tests drive it |

## Wallet UX for the same jobs

| Job | Reference | Source | Taken for zwallet |
|---|---|---|---|
| Key switcher | MetaMask: the account selector at the top of the wallet; each account is listed there | [switching accounts](https://support.metamask.io/configure/accounts/switching-accounts-in-metamask/) (seen in search results, not opened: `[unverified]`) | the keys tab, and the open key on bar line 1. zwallet adds what MetaMask has no need of: opening a key in the TUI is not making it the agent's active key (`s`, TUI-2) |
| Receive | Ledger Live: "Following device verification, you will be shown a receive address and QR code"; without the device it "does not have the added security protection" | [Ledger feature focus](https://www.ledger.com/ledger-live-feature-focus-onboarding-and-transactions) | the receive tab: full EIP-55 address, its trust words, a QR only when verified |
| Receive | Sparrow: "The address field will display the first receive address"; confirm it on the Coldcard with the Address Explorer | [Sparrow Coldcard guide](https://sparrowwallet.com/docs/coldcard-wallet.html) | `V` (password check) is zwallet's "confirm on the device" |
| Send | Ledger Live: recipient (typed, pasted or scanned; "double check the address"), amount, fees (High, Medium, Low, Custom), a summary of the total, then confirm on the device | [Ledger feature focus](https://www.ledger.com/ledger-live-feature-focus-onboarding-and-transactions) | payee from the checked list only (no typed address), amount, review (fee, cap, verdict, guards), type the nickname |
| Send | Sparrow: destination, label and amount, "Create Transaction", "Finalize Transaction for Signing", sign, "Broadcast Transaction" | [Sparrow Coldcard guide](https://sparrowwallet.com/docs/coldcard-wallet.html) | the review is its own step and signs nothing (03-tui.md §Status of a send) |
| Address book | Rabby: "Whitelist" against wrong and look-alike addresses; "This address is not whitelisted" before signing | [Rabby on X, 1](https://x.com/Rabby_io/status/1595757282247294976), [2](https://x.com/Rabby_io/status/1613494065416536072) (search-result text, posts not opened: `[unverified]`) | the whitelist tab; the daemon enforces it, so it is a gate, not a reminder |
| Stuck send | MetaMask: speed up re-uses the nonce with a higher fee; cancel sends to your own address at that nonce, any value; at least +10% priority fee, +30% max fee | [speed up or cancel](https://support.metamask.io/manage-crypto/transactions/how-to-speed-up-or-cancel-a-pending-transaction/) | `s` and `x` on a pending send of this machine (`--nonce`, `--replace`, `--bump` 1.25, README.md:594-622) |
| Signing view | Frame: calldata decoded into readable summaries with verified ABIs; "sign transactions with confidence" (repository archived) | [Frame](https://github.com/floating/frame) | the invoke review shows zwallet's own decode (contract, selector, value), not the explorer's `method` (03-tui.md §History keeps that out) |
| Lock state | MetaMask, Ledger: lock is a global state shown once | the sources above | already zwallet's bar line 2 (03-tui.md §Status bar) |

Not taken: WalletConnect, swaps and dapp connections (Rabby, MetaMask, Frame). zwallet's CLI has
none of them (README.md:51-85), and the TUI covers what the CLI does.

## ratatui widgets against zwallet's cell rule

Rule 1 of 03-tui.md lets a cell hold only printable ASCII or `₮`; `FilteredBackend` turns any
other cell into `?` and the tests require zero such cells. So a widget's **default** glyphs
matter:

| Widget | Default it draws | Source (ratatui-widgets 0.3.2 / ratatui-core 0.1.2) | Use in zwallet |
|---|---|---|---|
| `Block` borders | `BorderType::Plain`: `┌ ─ ┐ │ └ ┘` | widgets block.rs:278; core symbols/line.rs:1, :11, :26 | only with the ASCII set `+`, `-`, `\|` (src/tui/ui.rs:24-33, as today) |
| `Tabs` | divider `symbols::line::VERTICAL` = `│`; selected tab `REVERSED` | widgets tabs.rs:15, :143; core symbols/line.rs:1 | `.divider(" ")` or an ASCII string; selected style must be `style::SELECTED` (rule 2). The tab line can also be one `Row` of text, as the title line is now |
| `Scrollbar` | vertical `DOUBLE_VERTICAL`: track `║`, thumb `█`, `▲` `▼`; `VERTICAL`: `│`, `█`, `↑`, `↓` | widgets scrollbar.rs:183-189; core symbols/scrollbar.rs:20-25, :34-39 | not used; a list shows `N of M` in its title (lazygit's `showListFooter`) |
| `Gauge` | `symbols::block::FULL` = `█`, partial blocks with `use_unicode` | widgets gauge.rs:187-223; core symbols/block.rs:1 | not used |
| `LineGauge` | `symbols::line::HORIZONTAL` = `─` | widgets gauge.rs:287-288 | not used |
| `List` | no highlight symbol by default (`Option<Line>`) | widgets list.rs:121 | usable; the marker stays `>` |
| `Paragraph`, `Clear` | draw only the text given; `Clear` resets an area "for popups" | widgets clear.rs:1 | usable; popups are `Clear` then an ASCII-bordered `Block` in a centred rectangle, as ratatui's [popup example](https://ratatui.rs/examples/apps/popup/) does |

The docs.rs page for `Tabs` read on 2026-10-28 summarised the default divider as a pipe; the
source says `│` (U+2502). The source is what runs.

## What fits 80x24 and 120x40

Today's layout (src/tui/ui.rs:79-85): one title row, the pane, one hint row, two bar rows. With the
title row turned into the tab line nothing else changes:

| Terminal | Pane | Inside an ASCII border | Notes |
|---|---|---|---|
| 80x24 | 80 x 20 | 78 x 18 | one list of up to 16 rows plus a title and a note row; two lists stack (balances above activity) |
| 120x40 | 120 x 36 | 118 x 34 | two panes side by side (balances 48 wide, activity 70) |

- The tab line `zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session`
  is 77 columns: it fits 80.
- zwallet's QR of an EIP-55 address is version 3, 29 modules plus a quiet zone of 4 on each
  side: 37 columns and 19 lines in half blocks (src/qr.rs:119-122). Half blocks fail rule 1.
  Drawn in ASCII with one module as two spaces in a code-chosen background, it is 74 x 37 cells:
  it fits a 120x40 terminal as a full-screen overlay (37 rows, a caption row, a hint row), not
  80x24. Whether a phone reads a code drawn two cells wide per module is `[unverified]`; it
  depends on the terminal's font, whose cells are commonly about twice as tall as wide
  `[unverified]`.
