# zwallet — TUI screens (v2 mockups)

Status: **planned**, 2026-10-28, regenerated after review rounds 1 and 2 the same day. ASCII mockups for
[`03-tui.md`](03-tui.md) §Screens and keys: one per screen at 80x24 (the smallest size the TUI
draws), and the home screen at 120x40 too. They are generated with exact widths; every cell is
printable ASCII, as rule 1 asks, and the generator checks the hint rule on every frame (a frame
whose focus is not a text field, a menu or a popup ends its hint line with `? actions`; one whose
focus is shows no `? actions`; each heading below names a focus other than a pane). In a real
frame the selected tab and the selected row are drawn in `style::SELECTED` (reverse video); here
the selected tab is in `[ ]` and the selected row starts with `>`. Values are illustrative: the
addresses are the test keys of the dispatcher's 2026-10-28 frames (alpha, beta) and well-known
public addresses, the amounts are made up.

Layout of every frame: row 1 the tab line, rows 2-21 the pane (78x18 inside its border), row 22
the hint line of this screen, rows 23-24 the status bar (03-tui.md §Status bar, unchanged).
Popups are drawn over the pane (`Clear`, then an ASCII-bordered block).

## 1 keys

The keys tab after `V`: every key's address derived from the seed in this TUI, so each row says `verified` (rule 7). The per-row word comes with TUI-4 (T19): the name gives way to 17 so the address and the word stay whole (03-tui.md §Status bar). Before it, after a warm-up alone (TUI-2), the rows carry no word and TUI-2's note says what checks them; a key the bar shows `verified by session` is not flagged (TUI-2's note fix, 4f4d54f). `>` is the key open in the TUI, `*` the globally active key. T1 (merged), T2, T2b, T19.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ keys (* = active: the agent's default; > = open here) -----------------------+
|>* alpha              0  0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E  verified |
|   beta               1  0xa2Ead993Ce378f5B15C6445c9569f0Eb0a41f73F  verified |
|   savings            2  0x1b7E9f0C2d4A6b8C0e2F4a6B8c0D2e4F6a8B0c2D  verified |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|every address above was derived from the seed with the password (V, 14:02Z)   |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter open  s make active  n new key  V verify  A repair  B backup  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 1 keys: `n` new key (popup)

`create <name>` for a further key: the password derives it here (`verified: true`, README.md:103-110), an optional duration is `create --unlock`. The form says the new key becomes the active key, because `create` makes it so. T30.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ keys (* = active: the agent's default; > = open here) -----------------------+
|>* alpha              0  0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E  verified |
|   beta               1  0xa2Ead993Ce378f5B15C6445c9569f0Eb0a41f73F  verified |
|   savings            2  0x1b7E9f0C2d4A6b8C0e2F4a6B8c0D2e4F6a8B0c2D  verified |
|     + new key (create) ----------------------------------------------+       |
|     |name      > trading_         (a-z 0-9 _ -, at most 32)          |       |
|     |unlock    >                  (empty: no session)                |       |
|     |password  >                  (derives the key here: verified)   |       |
|     |                                                                |       |
|     |the new key becomes the active key: the agent's default         |       |
|     |Enter creates; Esc cancels                                      |       |
|     +----------------------------------------------------------------+       |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|every address above was derived from the seed with the password (V, 14:02Z)   |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Tab next field  Enter create  Esc cancel
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 1 keys: first run (text field)

A home without `wallet.json`: the keys tab is the `create` form for the master seed, `--words` included, the password typed twice (`read_password(.., true)`, commands.rs:702). Bar line 1 says `no wallet in this home` as today (src/tui/ui.rs:253-255). T31.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ no wallet in this home yet: create one --------------------------------------+
|home        /Users/z/.zwallet                                                 |
|first key   > main_          (a-z 0-9 _ -, at most 32)                        |
|words       > 24             (12 15 18 21 24)                                 |
|password    >                (at least 12 characters)                         |
|again       >                                                                 |
|                                                                              |
|this makes the master seed. Write the recovery phrase down before funding:    |
|B shows it once the wallet exists (the password again).                       |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Tab next field  Enter create  Esc quit the form
ethereum | no wallet in this home
locked | ethereum-rpc.publicnode.com
```

## 1 keys: `B` recovery phrase, option B2 only (text field)

Built only if the owner picks B2 (03-tui.md §Open trade-offs; recommended: B1). With B1, which TUI-10 builds, `B` shows the one line `run zwallet backup in a shell to see the recovery phrase`. B2's first condition: the purpose sentence above the password field. T32.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ recovery phrase (option B2: only if the owner picks it) ---------------------+
|this shows your recovery phrase; anything that can read this terminal reads it|
|                                                                              |
|password  > ************_                                                     |
|                                                                              |
|Enter shows the 24 words; Esc cancels and shows nothing                       |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
type the password  Enter show  Esc cancel
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 1 keys: the words, option B2 only (popup)

B2 only. The words are drawn by zwallet from `wallet::recovery_phrase`'s value after the password typed for this screen; `word` stands for each of them here. Any key hides them, and so do 30 s. No claim about scrollback is made for a terminal not checked by hand. T32.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ recovery phrase (24 words, BIP39 English, no passphrase) --------------------+
|this shows your recovery phrase; anything that can read this terminal reads it|
|                                                                              |
|  1. word      7. word     13. word     19. word                              |
|  2. word      8. word     14. word     20. word                              |
|  3. word      9. word     15. word     21. word                              |
|  4. word     10. word     16. word     22. word                              |
|  5. word     11. word     17. word     23. word                              |
|  6. word     12. word     18. word     24. word                              |
|                                                                              |
|write them down offline. Any key hides them; they hide by themselves in 30 s. |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
any key hides the words
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 2 wallet (80x24)

Home: the balances of the open key on the TUI's chain (native coin and the registry tokens with a balance; T5), and the last 10 rows of history (T3). Enter on an asset starts a send of it (3 send, step 2); Enter on a row opens the transaction (T4). `a` replaces the balances by every network (T15).

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ balances: alpha on ethereum (a: every network) ------------------------------+
|> ETH        1.204381                                                         |
|  USDC       2500                                                             |
|  USDT       40.5                                                             |
|  zero, hidden: DAI, WETH                                                     |
|                                                                              |
+------------------------------------------------------------------------------+
+ recent activity (10 of 23) --------------------------------------------------+
|> pending  10-28 14:02Z  out  0.25 ETH      alice         journal             |
|  mined    10-28 13:40Z  in   250 USDC      0x9f2c...81Ab explorer            |
|  mined    10-27 09:12Z  out  1000 USDC     bob           journal explorer    |
|  reverted 10-26 22:05Z  out  0 ETH         router        journal explorer    |
|  mined    10-26 18:31Z  in   0.5 ETH       0x44aE...0c19 explorer            |
|  mined    10-25 11:02Z  out  40 USDT       alice         journal explorer    |
|  mined    10-24 08:47Z  in   1 ETH         0x7d1B...e2F0 explorer            |
|  mined    10-23 16:20Z  out  0.1 ETH       bob           journal explorer    |
|  mined    10-22 10:05Z  in   300 USDC      0x9f2c...81Ab explorer            |
|  mined    10-21 19:44Z  out  5 USDC        alice         journal explorer    |
|ethereum: explorer and this machine's sends; 2 explorer rows not shown        |
+------------------------------------------------------------------------------+
Enter open  Tab pane  a every network  / look up hash  c chain  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 2 wallet (120x40)

At 120x40 the two panes sit side by side and the every-network list fits under the balances.

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ balances: alpha on ethereum -----------------++ recent activity (10 of 23) ------------------------------------------+
|> ETH        1.204381                         ||> pending  10-28 14:02Z out  0.25 ETH   alice         journal         |
|  USDC       2500                             ||  mined    10-28 13:40Z in   250 USDC   0x9f2c...81Ab explorer        |
|  USDT       40.5                             ||  mined    10-27 09:12Z out  1000 USDC  bob           journal explorer|
|  zero, hidden: DAI, WETH                     ||  reverted 10-26 22:05Z out  0 ETH      router        journal explorer|
|                                              ||  mined    10-26 18:31Z in   0.5 ETH    0x44aE...0c19 explorer        |
|every network (a):                            ||  mined    10-25 11:02Z out  40 USDT    alice         journal explorer|
|  ethereum   ETH 1.204381 USDC 2500 USDT 40.5 ||  mined    10-24 08:47Z in   1 ETH      0x7d1B...e2F0 explorer        |
|  base       ETH 0.031                        ||  mined    10-23 16:20Z out  0.1 ETH    bob           journal explorer|
|  arbitrum   ETH 0                            ||  mined    10-22 10:05Z in   300 USDC   0x9f2c...81Ab explorer        |
|  optimism   error: HTTP 429 from optimis...  ||  mined    10-21 19:44Z out  5 USDC     alice         journal explorer|
|  polygon    POL 12.5                         ||                                                                      |
|  bsc        BNB 0                            ||ethereum: explorer and this machine's sends; 2 explorer rows not shown|
|  avalanche  AVAX 0                           ||(not in the shape zwallet reads)                                      |
|  sepolia    ETH 0.5                          ||                                                                      |
|5 of 8 networks have a balance;               ||                                                                      |
|1 did not answer                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
|                                              ||                                                                      |
+----------------------------------------------++----------------------------------------------------------------------+
Enter open  Tab pane  a every network  / look up hash  c chain  [ ] tabs  q quit  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## `c` chain picker (popup)

`zwallet chains` as a list: key, name, symbol, chain id, RPC host (the host only, rule 3). Picking one reloads every pane for that chain (T8); nothing is stored. `e` sets an RPC URL for the highlighted chain.

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ balances: alpha on ethereum (a: every network) ------------------------------+
|> ETH        1.204381                                                         |
|+ chain (the TUI's own; the agent's default is unchanged) -------------------+|
||  key        name              symbol  chain id  rpc host                   ||
||> ethereum   Ethereum          ETH     1         ethereum-rpc.publicnode.com||
||  base       Base              ETH     8453      base-rpc.publicnode.com    ||
+|  arbitrum   Arbitrum One      ETH     42161     arbitrum-one-rpc.publicn...|+
+|  optimism   OP Mainnet        ETH     10        optimism-rpc.publicnode.com|+
||  polygon    Polygon PoS       POL     137       polygon-bor-rpc.publicno...||
||  bsc        BNB Smart Chain   BNB     56        bsc-rpc.publicnode.com     ||
||  avalanche  Avalanche C       AVAX    43114     avalanche-c-chain-rpc.pu...||
||  sepolia    Sepolia           ETH     11155111  ethereum-sepolia-rpc.pub...||
||  tron       (05-tron.md)                                                   ||
|+----------------------------------------------------------------------------+|
|  mined    10-24 08:47Z  in   1 ETH         0x7d1B...e2F0 explorer            |
|  mined    10-23 16:20Z  out  0.1 ETH       bob           journal explorer    |
|  mined    10-22 10:05Z  in   300 USDC      0x9f2c...81Ab explorer            |
|  mined    10-21 19:44Z  out  5 USDC        alice         journal explorer    |
|ethereum: explorer and this machine's sends; 2 explorer rows not shown        |
+------------------------------------------------------------------------------+
j/k move  Enter switch  e its RPC  Esc close
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## `c` then `e`: an RPC URL for this session (popup)

`--rpc <url>` in the TUI (§Coverage matrix row 20): the URL for the highlighted chain, for this session only, through the CLI's resolver; the field draws the scheme and host as typed and the rest as `*`; the chain id is checked before use. T42.

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ balances: alpha on ethereum (a: every network) ------------------------------+
|> ETH        1.204381                                                         |
|+ chain (the TUI's own; the agent's default is unchanged) -------------------+|
||  key        name              symbol  chain id  rpc host                   ||
||  ethereum   Ethereum          ETH     1         ethereum-rpc.publicnode.com||
||> base       Base              ETH     8453      base-rpc.publicnode.com    ||
+|+ RPC for base, this TUI session only -------------------------------------+|+
+||url  > https://base-mainnet.example-node.io/*****************_            ||+
|||the path and the query are drawn as *: they may hold an API key           |||
|||the node must answer chain id 8453 before it is used                      |||
|||nothing is saved: after a restart, ZWALLET_RPC_BASE or the default        |||
|||                                                                          |||
|||Enter uses it; Esc keeps the one in use                                   |||
|++--------------------------------------------------------------------------++|
|  mined    10-24 08:47Z  in   1 ETH         0x7d1B...e2F0 explorer            |
|  mined    10-23 16:20Z  out  0.1 ETH       bob           journal explorer    |
|  mined    10-22 10:05Z  in   300 USDC      0x9f2c...81Ab explorer            |
|  mined    10-21 19:44Z  out  5 USDC        alice         journal explorer    |
|ethereum: explorer and this machine's sends; 2 explorer rows not shown        |
+------------------------------------------------------------------------------+
type the URL  Enter use  Esc cancel
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: payee

Step 2 of 5 (step 1 is the asset, skipped when the send starts from a balance row). Only the checked list's address entries that cover this chain (T6b); a typed address is not accepted, and an entry at the zero address is never offered (T44). A new payee is added on 5 whitelist.

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send 2/5: payee for ETH on ethereum -----------------------------------------+
|asset   ETH, balance 1.204381 (Tab: another asset)                            |
|                                                                              |
|the checked list's address entries on ethereum (gen 4, checked 14:02Z):       |
|> alice      0x758a863FA27ffFc71686d4a67B7c45e79D0235FE  ethereum, base       |
|  bob        0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045  *                    |
|                                                                              |
|not offered here: 1 entry on base only, 2 contract entries                    |
|a payee that is not listed cannot be typed here; list it on 5 whitelist (a)   |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
j/k move  Enter pick  Tab asset  Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: amount (text field)

The amount field takes `0-9` and one `.` (rule 3). The focus is the field, so the hint line has no `? actions`: `?` would be typed (and refused by the field's alphabet). The decimals come from the registry (or the chain's coin).

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send 3/5: amount ------------------------------------------------------------+
|to       alice  0x758a863FA27ffFc71686d4a67B7c45e79D0235FE                    |
|asset    ETH on ethereum, balance 1.204381                                    |
|                                                                              |
|amount > 0.25_                                                                |
|         digits and one '.', at most 18 decimals (ETH)                        |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
type the amount  Enter review  Esc back
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: review

The review signs nothing and takes no lock (03-tui.md §Status of a send). Here the pending guard fails: the message and the override key `p` (`--allow-pending`, T21); `d` is `--allow-duplicate`, `g` sets `--max-fee-gwei` and re-runs the review (T22).

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send 4/5: review (nothing is signed yet) ------------------------------------+
|to          alice                                                             |
|            0x758a863FA27ffFc71686d4a67B7c45e79D0235FE                        |
|amount      0.25 ETH on ethereum (chain id 1)                                 |
|from        alpha 0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                  |
|nonce       13                                                                |
|fee         at most 0.000231 ETH (21000 gas x 11 gwei max)                    |
|fee cap     0.003 ETH (the checked list, gen 4)                               |
|whitelist   allowed: alice is a payee on ethereum                             |
|funds       enough: balance 1.204381 ETH                                      |
|pending     nonce 12 is pending (0x3f1c...e2a0, journal 13:58Z)               |
|            p sends anyway: only for a different payment                      |
|repeat      none in the last 10 minutes                                       |
|                                                                              |
|this review is held: confirm signs only this payee, amount and nonce,         |
|and only if the fee is at most the one above                                  |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter confirm  p pending ok  d repeat ok  g fee ceiling  Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: confirm (popup)

Confirm by typing the payee's nickname (T6).

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send 4/5: review (nothing is signed yet) ------------------------------------+
|to          alice                                                             |
|            0x758a863FA27ffFc71686d4a67B7c45e79D0235FE                        |
|amount      0.25 ETH on ethereum (chain id 1)                                 |
|from        alpha 0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                  |
|nonce       13                                                                |
|fee        + confirm ---------------------------------------------+           |
|           |type the payee's nickname to sign and send            |           |
|           |0.25 ETH to alice on ethereum:                        |           |
|           |                                                      |           |
|           |> ali_                                                |           |
|           |                                                      |           |
|           |Enter sends once the name matches; Esc goes back      |           |
|           +------------------------------------------------------+           |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
type alice  Enter send  Esc back
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: status

The hash at once, then the status every 3 s; after 90 s `unknown - run zwallet tx`, and `w` with a typed duration keeps polling (T43). `s` speeds the send up and `x` cancels it, both as a replacement at its nonce, offered only with a pinned own address and an intent the node confirms (T27, T28); after `BROADCAST_UNCERTAIN` the screen offers `R`, a retry at the same nonce (T29).

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send 5/5: status ------------------------------------------------------------+
|hash                                                                          |
|  0x5be1f0a7c3d29e8841b6f2a0c9d3e7b5a1f4c8e2d6b0a9f3c7e1d5b9a2f6c0e4          |
|status      pending, read every 3 s (0:21 so far; after 1:30 it says unknown) |
|sent        0.25 ETH to alice on ethereum, nonce 13, from alpha               |
|journal     written before the broadcast (sends-1-0xcde5....jsonl)            |
|explorer    https://etherscan.io/tx/<the hash above>                          |
|                                                                              |
|leaving this screen cancels nothing; the send is in the journal               |
|s speeds it up and x cancels it, both at nonce 13 (a replacement)             |
|w keeps waiting for a duration you type (now: 90 s, as zwallet's --wait)      |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter details  w keep waiting  s speed up  x cancel  Esc wallet  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 3 send: `x` cancel, review

A cancel is 0 of the coin to the key's own pinned address at nonce N, under the checked list's fee cap, offered under the one intent rule; the review says it races the original (03-tui.md §Status of a send, Replacements). `b` sets the bump and runs the review again. Confirm by typing `cancel`. T28.

```text
zwallet  1 keys  2 wallet [3 send] 4 receive  5 whitelist  6 tools  7 session
+ send: cancel nonce 13, review (nothing is signed yet) -----------------------+
|replaces    0x5be1...c0e4, nonce 13, pending; the node's transaction          |
|            matches the journal row                                           |
|sends       0 ETH to alpha's own address, pinned by the password check:       |
|            0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                        |
|fee         at most 0.000289 ETH (21000 gas x 13.75 gwei max, bump 1.25)      |
|fee cap     0.003 ETH (the checked list, gen 4)                               |
|                                                                              |
|a cancel replaces nonce 13 only if it is mined first; if the original is      |
|mined first, the payment stands and the cancel fails                          |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter confirm (type cancel)  b bump  Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## transaction detail

From a history row, the send status, or `/` (a hash typed in: `tx`, T26). Fields from the node, token movements from the receipt, nicknames only from the checked list (T4).

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ transaction (from the node) -------------------------------------------------+
|hash      0x3f1c88b2a7e4d9c0f5b1a6e3d8c2f7b4a9e0d5c1f6b3a8e4d9c0f5b1a6e3e2a0  |
|status    mined, success; block 21480112, 37 confirmations                    |
|from      0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E (alpha)                  |
|to        0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD router                   |
|nonce     11      value 0 ETH                                                 |
|fee       0.000412 ETH (98211 gas x 4.19 gwei)                                |
|token movements (from the receipt):                                           |
|  1000 USDC 0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                        |
|         -> 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045 bob                    |
|  not shown: 1 log of a token outside the registry                            |
|local record: invoke, nonce 11, 10-27 09:12:04Z (this machine's journal)      |
|explorer  https://etherscan.io/tx/<the hash above>                            |
|nicknames come only from the list checked with the password (gen 4)           |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 4 receive

The open key's address on the TUI's chain, in full EIP-55, with its trust words (T17). `V` runs the password check when it is not verified. The Tron line follows 05-tron.md (T38).

```text
zwallet  1 keys  2 wallet  3 send [4 receive] 5 whitelist  6 tools  7 session
+ receive: alpha on ethereum --------------------------------------------------+
|address                                                                       |
|  0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                                  |
|  verified by password (derived from the seed in this TUI, 14:02Z)            |
|                                                                              |
|the same address on every EVM network in zwallet chains:                      |
|  ethereum base arbitrum optimism polygon bsc avalanche sepolia               |
|tron (05-tron.md)                                                             |
|  T... per 05-tron.md, with its own trust words                               |
|                                                                              |
|Q shows it as a QR code (a 120x40 terminal or larger)                         |
|send to it only an asset of the network you picked (c)                        |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Q QR code  V verify  c chain  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 4 receive: `Q` QR code at 120x40, option Q3 only (overlay)

Only if the owner picks Q3 (03-tui.md §Open trade-offs; recommended). Until then `Q` shows the one line `zwallet address shows it as a QR code` (Q1). With Q3: a full-screen overlay at 120x40 or larger; the dotted block marks where the 74x37 cells of the code go (each module two spaces in `QR_DARK` or `QR_LIGHT`); at 80x24 `Q` says `the QR code needs a 120x40 terminal; zwallet address shows one`. T18.

```text
scan to send to 0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E  (the same address on every EVM network)
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ........  placeholder: zwallet's QR of the verified address       ........
                       ........  37 x 37 modules (version 3, quiet zone 4), each         ........
                       ........  module two spaces in QR_DARK or QR_LIGHT: 74 x 37       ........
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
                       ..........................................................................
any key closes   the QR is drawn only for an address verified by password or by session
```

## 5 whitelist

The list as the password check verified it (or `not verified` with `V` offered, 03-tui.md §Password check), its policy, and the session's view as information only (T7). `z`, a signed entry at the zero address from before TUI-6, is flagged, never offered as a payee, and `d` removes it (T44).

```text
zwallet  1 keys  2 wallet  3 send  4 receive [5 whitelist] 6 tools  7 session
+ whitelist gen 4: verified (checked with the password, 14:02Z) ---------------+
|  nickname kind      address                                     chains       |
|> alice    address   0x758a863FA27ffFc71686d4a67B7c45e79D0235FE  ethereum,base|
|  bob      address   0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045  *            |
|  router   contract  0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD  ethereum     |
|  z        address   0x0000000000000000000000000000000000000000  ethereum     |
|z is the zero address: never offered as a payee; d removes it                 |
|fee caps    ethereum 0.003 ETH, base 0.0005 ETH, (6 more: Enter on policy)    |
|messages    sign-message is off                                               |
|session     the running session reports gen 4, 3 entries (not checked here)   |
|                                                                              |
|every change asks for the password, stops every zwallet daemon of this user,  |
|and leaves the wallet locked unless you give a duration with it               |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
a add  d remove  f fee cap  m messages  R reset  V check  Enter send  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 5 whitelist: `a` add (text field)

`whitelist add`: the kind (address or contract with selectors, T24), nickname, the full address typed (never pre-filled; the zero address refused, T44), chains picked from a list (`*` included), and a duration that becomes `--unlock`.

```text
zwallet  1 keys  2 wallet  3 send  4 receive [5 whitelist] 6 tools  7 session
+ whitelist: add an entry -----------------------------------------------------+
|kind        > address    contract          (Tab)                              |
|nickname    carol_                                                            |
|address     0x                                    (full address, typed)       |
|chains      [x] ethereum [ ] base [ ] arbitrum [ ] optimism [ ] polygon       |
|            [ ] bsc [ ] avalanche [ ] sepolia [ ] * every chain               |
|selectors   (contract only)                                                   |
|unlock for  (empty: the wallet stays locked after the change)                 |
|                                                                              |
|nothing is pre-filled from history; the address is never taken from a row     |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Tab next field  space toggle  Enter summary  Esc cancel
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 5 whitelist: summary and password (popup)

`wallet::whitelist_plan`'s summary above the password field (03-tui.md §Whitelist edits). `d` remove, `f` fee cap, `m` message signing and `R` reset use the same popup (T23, T25).

```text
zwallet  1 keys  2 wallet  3 send  4 receive [5 whitelist] 6 tools  7 session
+ whitelist: add an entry -----------------------------------------------------+
|kind        > address    contract          (Tab)                              |
|nickname    carol_                                                            |
|address     0x                                    (full address, typed)       |
|ch+ change: add carol (password for this change) --------------------------+  |
|  |add carol: payee 0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed             |  |
|se|  on ethereum                                                           |  |
|un|lands on whitelist.json (gen 4, the sealed list) as gen 5               |  |
|  |every zwallet daemon of this user is stopped (2 running)                |  |
|no|after the change: the wallet stays locked                               |  |
|  |                                                                        |  |
|  |add carol on ethereum; the wallet stays locked                          |  |
|  |password > ************_                                                |  |
|  +------------------------------------------------------------------------+  |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter apply  Esc cancel (nothing changes)
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 6 tools

The functions that are not about one asset: `call`, `invoke`, `sign-message`, `tx`, `balance --token`.

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist [6 tools] 7 session
+ tools -----------------------------------------------------------------------+
|> call      read a contract (eth_call); nothing is signed     zwallet call    |
|  invoke    call a listed contract with a listed selector      invoke         |
|  sign      sign a message (EIP-191) if the list allows it     sign-message   |
|  tx        look a transaction up by its hash                  tx             |
|  token     the balance of any ERC-20 by its contract          balance --token|
|                                                                              |
|invoke and sign need a warm-up and the list checked with the password (V)     |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
j/k move  Enter open  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 6 tools: call (text field)

`call`: a contract address, a signature and arguments typed in; nothing is signed, so the address may be any (T33).

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist [6 tools] 7 session
+ tools: call (eth_call on ethereum, nothing is signed) -----------------------+
|contract    0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48                        |
|function    balanceOf(address)(uint256)                                       |
|arguments   0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                        |
|from        alpha (the open key)                                              |
|                                                                              |
|result      2500000000                                                        |
|raw         0x000000000000000000000000000000000000000000000000000000009502f900|
|                                                                              |
|the result is the contract's own text, escaped and cut to 512 characters      |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Tab next field  Enter call  Esc back
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 6 tools: invoke review

`invoke`: the contract is picked from the checked list's contract entries on this chain, the selector from that entry's selectors; the signature (or raw calldata) must match a listed selector. The review shows zwallet's own decode; confirm by typing the contract's nickname (T34, T35). Here the pending guard fails, so the hint offers `p` (`--allow-pending`); `d` appears when the repeat guard fails. A speed-up or retry of an invoke is judged by the same checks (03-tui.md §Status of a send, Replacements).

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist [6 tools] 7 session
+ tools: invoke 3/4: review (nothing is signed yet) ---------------------------+
|contract    router (a contract entry on ethereum)                             |
|            0x3fC91A3afd70395Cd496C647d5a6CC9D4B2b7FAD                        |
|selector    0x414bf389 exactInputSingle(...) (listed for router)              |
|calldata    0x414bf389 + 256 bytes (Enter on this line shows it all)          |
|value       0 ETH                                                             |
|decoded     contract-call: contract router, selector 0x414bf389, value 0      |
|nonce       13                                                                |
|fee         at most 0.00094 ETH (cap 0.003 ETH, gen 4)                        |
|whitelist   allowed: router lists 0x414bf389 on ethereum                      |
|pending     nonce 12 is pending (0x3f1c...e2a0, journal 13:58Z)               |
|            p sends anyway: only for a different call                         |
|a listed contract may forward value or act on its arguments                   |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter confirm (type router)  p pending ok  g fee ceiling  Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 6 tools: sign a message, review

`sign-message`: rule 4's message block shows the whole message, here 512 characters, 76 to a row between `[` and `]`, so a space at either end of a row is visible; the line under it counts the bytes and the leading and trailing spaces. `e` goes back to the field and drops the review. Confirm by typing `sign`. With message signing off, the review shows the refusal and points to `m` on 5 whitelist (T36).

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist [6 tools] 7 session
+ tools: sign 2/3: review as alpha (nothing is signed yet) --------------------+
|the whole message, 76 characters a row, between [ and ]:                      |
|[login to example.org as alpha; nonce 8812; issued 2026-10-28T14:05Z; login t]|
|[o example.org as alpha; nonce 8812; issued 2026-10-28T14:05Z; login to examp]|
|[le.org as alpha; nonce 8812; issued 2026-10-28T14:05Z; login to example.org ]|
|[as alpha; nonce 8812; issued 2026-10-28T14:05Z; login to example.org as alph]|
|[a; nonce 8812; issued 2026-10-28T14:05Z; login to example.org as alpha; nonc]|
|[e 8812; issued 2026-10-28T14:05Z; login to example.org as alpha; nonce 8812;]|
|[ issued 2026-10-28T14:05Z; login to example.org as alpha]                    |
|512 bytes; leading spaces 0, trailing spaces 0                                |
|signer      alpha 0xCDe5267162DDaBFb9e61a1B1deF841760B799E7E                  |
|whitelist   message signing is on (the checked list, gen 4)                   |
|a signature can stand for you off chain: sign only what you wrote             |
|e edits the message, which drops this review                                  |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
Enter confirm (type sign)  e edit  Esc back  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 7 session

`status` in full (T16), `u` to unlock for a typed duration (T14), `L` to lock, the chains. The `ptrace` line appears only when `zwallet tui --allow-ptrace-scope-0` started the TUI on Linux: no key sets it (03-tui.md §Coverage matrix row 8, T20).

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist  6 tools [7 session]
+ session ---------------------------------------------------------------------+
|lock        signing (agent too) until 2026-10-28 14:32Z, in 30 min            |
|daemon      pid 51234, this build's key daemon of this home                   |
|other homes 1 daemon (pid 4411); L stops it too                               |
|whitelist   the running session reports gen 4, 3 entries (not checked here)   |
|home        /Users/z/.zwallet                                                 |
|ptrace      ptrace_scope 0 accepted (started with --allow-ptrace-scope-0)     |
|version     zwallet 0.2.0 (preview 24f4b5b0c1d2)                              |
|                                                                              |
|chains      ethereum base arbitrum optimism polygon bsc avalanche sepolia     |
|            c shows each one's name, symbol, chain id and RPC host            |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
u unlock for...  L lock every zwallet daemon  c chain  ? actions
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## 7 session: `u` unlock for a duration (popup)

`unlock <dur>`: the duration, then the password (the TUI-2 password field). The `ptrace_scope 0 accepted` line appears only under the start flag, as in the warm-up's password popup (T20).

```text
zwallet  1 keys  2 wallet  3 send  4 receive  5 whitelist  6 tools [7 session]
+ session ---------------------------------------------------------------------+
|lock        locked                                                            |
|daemon      none for this home                                                |
|                                                                              |
|                                                                              |
|      + unlock (signing opens for the agent too) ----------------------+      |
|      |for       > 24h_          (s m h d w y; 1y = 365d)              |      |
|      |password  >                                                     |      |
|      |                                                                |      |
|      |ptrace_scope 0 accepted (started with --allow-ptrace-scope-0)   |      |
|      |Enter unlocks; Esc cancels                                      |      |
|      +----------------------------------------------------------------+      |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
|                                                                              |
+------------------------------------------------------------------------------+
type a duration, then the password  Esc cancel
ethereum | alpha 0xCDe5...9E7E | not verified
locked | ethereum-rpc.publicnode.com
```

## `?` or `:` action menu

One row per §Coverage matrix row 1–52, so `actions (52)`: its key, what it does, its alias token (the matrix's Alias column) and its tab, or why it cannot run now (`start flag` for row 8, whose Enter only explains it; `CLI pointer` for B1). Typing filters, Enter runs (T12). The navigation keys and the version are on its last lines and are not counted.

```text
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
+ keys (* = active: the agent's default; > = open here) -----------------------+
|>+ actions (52): type to filter, Enter runs, Esc closes --------------------+ |
| |> _                                                                       | |
| |  1     the key list          list                           1            | |
| |  V     verify every address  list --verify                  1            | |
| |  -     create the wallet     create                         first run    | |
| |  n     new key               create                         1            | |
| |  n     new key, then unlock  create --unlock                1            | |
| |  s     make the key active   change                         1            | |
| |  Enter warm up / unlock      unlock                         1; 7 u       | |
| |  -     ptrace_scope 0: why   unlock --allow-ptrace-scope-0  start flag   | |
| |  L     lock every daemon     lock                           any tab      | |
| |  7     the session           status                         7            | |
| |  B     recovery phrase       backup                         CLI pointer  | |
| |  ...   (j/k scrolls; 41 more)                                            | |
| |keys: 1-7 tabs  [ ] tabs  j/k move  r refresh  Esc back  q quit           | |
|e|zwallet 0.2.0 (preview 24f4b5b0c1d2)                                      | |
| +--------------------------------------------------------------------------+ |
|                                                                              |
+------------------------------------------------------------------------------+
type to filter  j/k move  Enter run  Esc close
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```

## `?` action menu, filtered

Typing `fee` leaves the two rows whose words or alias hold it; Enter runs the selected one, which opens its screen or form.

```text
zwallet  1 keys [2 wallet] 3 send  4 receive  5 whitelist  6 tools  7 session
+ balances: alpha on ethereum (a: every network) ------------------------------+
|> ETH        1.204381                                                         |
|  USDC       2500                                                             |
|  US+ actions: type to filter, Enter runs, Esc closes -------------------+    |
|  ze|> fee_                                                              |    |
|    |  g   fee ceiling for a send   transfer --max-fee-gwei   3          |    |
+----|  f   a chain's fee cap        whitelist fee-cap         5          |----+
+ rec|                                                                    |----+
|> pe|2 of 52 actions match                                               |    |
|  mi+--------------------------------------------------------------------+    |
|  mined    10-27 09:12Z  out  1000 USDC     bob           journal explorer    |
|  reverted 10-26 22:05Z  out  0 ETH         router        journal explorer    |
|  mined    10-26 18:31Z  in   0.5 ETH       0x44aE...0c19 explorer            |
|  mined    10-25 11:02Z  out  40 USDT       alice         journal explorer    |
|  mined    10-24 08:47Z  in   1 ETH         0x7d1B...e2F0 explorer            |
|  mined    10-23 16:20Z  out  0.1 ETH       bob           journal explorer    |
|  mined    10-22 10:05Z  in   300 USDC      0x9f2c...81Ab explorer            |
|  mined    10-21 19:44Z  out  5 USDC        alice         journal explorer    |
|ethereum: explorer and this machine's sends; 2 explorer rows not shown        |
+------------------------------------------------------------------------------+
type to filter  Enter run  Esc close
ethereum | alpha 0xCDe5...9E7E | verified by password
signing (agent too) until 14:32Z | ethereum-rpc.publicnode.com
```
