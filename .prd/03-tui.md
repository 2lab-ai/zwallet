# zwallet — TUI

Status: **planned** (v0.2), revised 2026-10-20 after two spec reviews, revised 2026-10-27 against
PRs #6–#12 (`origin/main` c7d3f13) and after review rounds 1 and 2 the same day, on 2026-10-28
for TUI-1b (§Changes on 2026-10-28), and the same day, against `origin/main` 24f4b5b, to cover
every CLI function (§Changes on 2026-10-28, v2: the coverage matrix, the re-decided non-goals,
the navigation model; mockups in [`03-tui-screens.md`](03-tui-screens.md), research in
[`research/tui-ux.md`](research/tui-ux.md)). The signed
whitelist shipped in v0.1 (WU1b, PR #4, [`04-whitelist.md`](04-whitelist.md)). TUI-1 starts once the
agent-side v0.1 work is merged; G3 and G4 wait on owner actions only and do not block TUI code
([`v0-2-tui/loop.md`](v0-2-tui/loop.md) §Start condition). Fixed user statement:
[`v0-2-tui/ssot.md`](v0-2-tui/ssot.md). What the TUI would call today, function by function:
[`v0-2-tui/flow.md`](v0-2-tui/flow.md). Units and order: [`v0-2-tui/units.md`](v0-2-tui/units.md).

## Problem

Every zwallet command is a one-shot CLI call with JSON output, which suits an agent. A human who
wants to check the wallet, approve a send or curate payees has to remember flags and read JSON.
On 2026-10-20 the user asked for `zwallet tui` to do the following: list the wallets, warm one up
with the password for about an hour, show the balance and the last 10 transactions, show a
transaction in detail, transfer an asset to a whitelisted payee, and check the result by tx hash.

On 2026-10-28 the owner saw the first preview (TUI-1b: the key list and the lock state; the hint
line offered only `j/k`, `r`, `L` and `q`) and wrote: "tui로 시발 뭐 아무것도 못하는데 tui로 모든 기능
다 사용할수 있도록 ui에 대해서 리설치하고 tui로 모든 기능 사용할 수 있도록 구현" (research the UI, then make
every feature usable from the TUI; [`v0-2-tui/ssot.md`](v0-2-tui/ssot.md)). The same day he asked
for a Tron family, which the TUI covers too. So the target is no longer the 2026-10-20 list
alone: it is every CLI function (§Coverage matrix), each reachable from a screen and findable by
a first-time user (§Screens and keys).

## Goals

- **T1 Keys.** Opens on the key list (the master's named subkeys), with the globally active key
  marked and each row showing its address. Lock state is shown once, in the status bar, because it
  belongs to the master and not to a key. The row addresses are never marked verified (`list`
  without the password is never verified, flow.md §T1).
- **T2 Warm-up.** Enter opens a key *in the TUI* (an internal `--key`, not a global switch). If the
  wallet is locked, it asks for the password and unlocks for **1h** through the staged unlock.
  `zwallet tui --warm <dur>` changes the 1h; `zwallet tui` and `--warm` are new (flow.md Fact 5).
  With the same password the TUI also opens `wallet.json` itself, as `list --verify` and
  `whitelist list --verify` do: it derives the open key's address from the seed and verifies the
  whitelist (§Password check). Making the key globally active is a separate key press (`s`) with
  its own confirmation, because the active key is what the agent uses by default.
- **T3 Home.** The balance summary, plus the last 10 transactions for the current chain (§History).
- **T4 Tx detail.**
  - Fields: hash, the transaction's own from/to, nonce, native value, every registry-token
    movement that involves the wallet (from, to, amount, read from the node's receipt), fee
    (`gasUsed × effectiveGasPrice`, plus the receipt's `l1Fee` on base and optimism), block,
    confirmations, status, explorer URL. Today's `tx` lookup gives none of the fee, the
    confirmations or the token movements (flow.md §T4).
  - A from or to address gets a nickname only from the password-checked whitelist
    (§Password check); otherwise the address stands alone.
  - For this machine's sends it also shows the send journal's own fields (kind, amount, recipient,
    nonce, time), labelled "local record".
- **T5 Assets.** Enter on the balance lists the native coin plus the registry tokens (USDT, USDC,
  DAI, WETH) with a non-zero balance.
- **T6 Transfer.** Enter on an asset starts a transfer:
  1. pick the recipient from the password-checked whitelist's **address entries valid on this
     chain** (§Password check);
  2. enter the amount;
  3. review the send;
  4. confirm by typing the payee's nickname;
  5. the tx hash appears at once, and the screen follows its status (§Status of a send).
- **T7 Whitelist.** List the entries (gen, `verified` state, policy), add an address entry or a
  contract entry with selectors, remove an entry, set the fee cap, reset. The `verified` state is
  the password check's (§Password check).
  - Every change asks for the password, even when warm, and runs the 04 edit protocol
    (§Whitelist edits).
  - The add screen shows the full EIP-55 address and never pre-fills an address from history
    (address-poisoning defence).
- **T8 Chain.** A chain switcher. The default is `--chain <key>` when given, otherwise ethereum.
- **Every function (2026-10-28).** Every CLI command and user-meaningful flag is reachable from a
  screen, or stays CLI-only for a security reason written out (§Coverage matrix). Every function
  is findable from any screen through the action menu (§Screens and keys).

## Non-goals (re-decided 2026-10-28)

Each non-goal of 2026-10-20 was weighed again against "모든 기능" ([`v0-2-tui/ssot.md`](v0-2-tui/ssot.md)
§User statement, 2026-10-28). One stays only for a security reason written out; what the CLI does
not have is not a TUI goal either.

| Non-goal (2026-10-20) | 2026-10-28 | Why |
|---|---|---|
| No mnemonic display and no backup screen | **open**: trade-off B (§Open trade-offs) | the TUI shows the phrase no more widely than `zwallet backup` does, but a second screen with a secret widens the one exception of zwallet's "never print secrets" rule, which is the owner's call |
| No free-form recipient field: no payee outside the whitelist, ever | **kept** | security: a payee typed or pasted at send time is where address poisoning and clipboard swaps land (§History drops the poisoning pattern for the same reason). The key daemon refuses an unlisted payee anyway (`WHITELIST_DENIED`, README.md:284-301), so such a field adds only refusals, or a second way to a listed address that skips its nickname. A new payee is listed on 5 whitelist (`a`): the password, the full address typed and shown in EIP-55, the summary (§Whitelist edits) |
| No `invoke` screen | **dropped**: invoke on 6 tools (T34, T35) | the key daemon judges every call against the contract entries and their selectors (README.md:294-301); the TUI offers only the checked list's contract entries on the chain and their selectors, so it adds no way past that judgement |
| No swaps, no WalletConnect | **kept** | not a CLI function (README.md:60-85 has neither); the TUI covers what the CLI does |
| No settings file | **kept** | not a CLI function, and a file in the home is writable by any process of this user (README.md:845-848): it could preselect a chain, a duration or a payee on the human's screen |
| No QR code | **open**: trade-off Q (§Open trade-offs) | the CLI draws its QR past the write filter (core/terminal.rs:6-7) in half blocks, which rule 1 turns into `?`; each way to draw one in the TUI costs something |
| No second implementation | **kept** | the TUI calls the library entry points the CLI calls ([`v0-2-tui/flow.md`](v0-2-tui/flow.md)); the send lock, the journal, the guards and the daemon's whitelist apply unchanged |
| allow-message-signing is not a TUI behaviour (flow.md §T7) | **dropped**: `m` on 5 whitelist (T25) | a whitelist change like `fee-cap`: the same summary, password and edit protocol |

What stays out of the TUI after this (rows 53–59 of §Coverage matrix): a typed payee (above),
`transfer --force` (the zero address burns funds), a message to sign that the review cannot show
whole, two sends the daemon never signs, and the flags that only make sense for a one-shot
command (`--json`, `--password-stdin`, `--no-qr`).

## Coverage matrix

Every CLI command and every user-meaningful flag (README.md §Commands, src/cli.rs:36-443 at
24f4b5b), and the Tron functions the owner asked for on 2026-10-28. "Screen" is the tab (§Screens
and keys); the CLI column is also each action's alias in the action menu, spelled as here, and
T12 checks the menu against rows 1–52. Mockups: [`03-tui-screens.md`](03-tui-screens.md).

| # | CLI function | Screen | Key(s) | Flow | Acceptance | Unit |
|---|---|---|---|---|---|---|
| 1 | `list` | 1 keys | `1` | the rows, `*` the active key, `>` the key open here | T1 | TUI-1b (merged) |
| 2 | `list --verify` | 1 keys | `V` | the password, then every key's address derived from the seed; each row says `verified` | T19 | TUI-4 |
| 3 | `create <name>` (first run, `--words`) | 1 keys | the tab is the form while the home has no wallet | name, word count, the password twice: the master seed and key 0; the backup reminder | T31 | TUI-10 |
| 4 | `create <name>` (another key) | 1 keys | `n` | name, the password: derived here, verified; it becomes the active key, and the form says so | T30 | TUI-10 |
| 5 | `create --unlock <dur>` | 1 keys | `n`, the duration field | as 3 or 4, then a session for that duration | T30 | TUI-10 |
| 6 | `change <name>` | 1 keys | `s`, then `y` | the key becomes the agent's default | T2b | TUI-2 |
| 7 | `unlock <dur>` | 1 keys; 7 session | Enter (the warm-up, `--warm`, 1h); `u` (a typed duration) | the password, then the session | T2, T14 | TUI-2, TUI-4 |
| 8 | `unlock --allow-ptrace-scope-0` | start | `zwallet tui --allow-ptrace-scope-0` | passed to every warm-up, unlock and duration given with a change in that TUI | T20 | TUI-4 |
| 9 | `lock` | every tab | `L` | every zwallet daemon of this user stopped; the bar says `locked` | T10 | TUI-1b, TUI-1c |
| 10 | `status` | 7 session; the bar | `7` | lock state, expiry, daemon pid, other homes' daemons, the session's whitelist view, the home | T16 | TUI-4 |
| 11 | `backup` | 1 keys | `B` | trade-off B | T32 | TUI-10 |
| 12 | `address [--chain]` | 4 receive | `4`, `c` | the full EIP-55 address and its trust words | T17 | TUI-4 |
| 13 | `address --verify` (repair) | 1 keys | `A` | the password, then the cached addresses repaired from the seed; the `repaired` list shown | T19 | TUI-4 |
| 14 | `address` QR code | 4 receive | `Q` | trade-off Q | T18 | TUI-4 |
| 15 | `addresses` | 4 receive | the list under the address | one address per family, every chain named | T17 | TUI-4 |
| 16 | `balance` (every network) | 2 wallet | `a` | every chain, the testnet last, a failed one on its own line | T15 | TUI-3 |
| 17 | `balance --chain` | 2 wallet | `2`, `c` | the native coin and the registry tokens with a balance | T5 | TUI-3 |
| 18 | `balance --token <erc20>` | 6 tools | token | a contract typed in: its balance, its own symbol escaped | T37 | TUI-11 |
| 19 | `chains` | chain picker | `c` | key, name, symbol, chain id, RPC host | T8 | TUI-3 |
| 20 | `--rpc <url>` | the bar | `ZWALLET_RPC_<CHAIN>` set before `zwallet tui` | the variable the CLI reads after `--rpc` (`resolve_rpc`, chains/mod.rs:178-186); line 2 shows its host | T8 | TUI-1b, TUI-3 |
| 21 | `transfer --to @nick --amount` | 3 send | `3`; Enter on a balance row | asset, payee, amount, review, the nickname typed, status | T6, T6b | TUI-5 |
| 22 | `transfer --token <registry token>` | 3 send | Enter on a token row | as 21 | T6 | TUI-5 |
| 23 | `--dry-run` | 3 send | the review | prepare, the guards, the verdict; no signature, no lock | T6 | TUI-5 |
| 24 | `--wait [--wait-timeout]` | 3 send | the status | polls every 3 s; after 90 s `unknown - run zwallet tx` | T6 | TUI-5 |
| 25 | `--allow-pending` | 3 send, review | `p` | its warning, then held by the review | T21 | TUI-5 |
| 26 | `--allow-duplicate` | 3 send, review | `d` | as 25 | T21 | TUI-5 |
| 27 | `--max-fee-gwei` | 3 send, review | `g` | a price per gas in gwei, then the review again | T22 | TUI-5 |
| 28 | `--nonce N --replace <hash> --bump <f>` (speed up) | send status; transaction | `s` | the same payment at nonce N, the fees bumped | T27 | TUI-9 |
| 29 | the same as a cancel (0 to the key itself) | send status; transaction | `x` | 0 of the coin to the key's own (pinned) address at nonce N; `cancel` typed. `parse_units` takes `0` ("a non-negative decimal", core/amount.rs:8-45) and the daemon signs a native self-transfer (README.md:285-286) | T28 | TUI-9 |
| 30 | `--nonce N --replace <hash>` (retry) | send status | `R` | after `BROADCAST_UNCERTAIN`: the same send, replacing only the first attempt | T29 | TUI-9 |
| 31 | `tx --chain <hash>` | 2 wallet; 6 tools | `/`; tx | a hash typed in, then the transaction | T26 | TUI-8 |
| 32 | `call` | 6 tools | call | contract, signature, arguments; the result | T33 | TUI-11 |
| 33 | `invoke --to @nick --sig … [--value]` | 6 tools | invoke | a contract entry, one of its selectors, the signature and arguments, a value; review; the nickname typed; status | T34 | TUI-11 |
| 34 | `invoke --data` | 6 tools | invoke, raw | calldata whose first 4 bytes are a listed selector | T35 | TUI-11 |
| 35 | `invoke` with the send flags | 6 tools | `p`, `d`, `g`, `s`, `x` | as 23–30 | T34 | TUI-11 |
| 36 | `sign-message` | 6 tools | sign | the message, the review, `sign` typed, the signature | T36 | TUI-11 |
| 37 | `whitelist list` | 5 whitelist | `5`, `w` | `not verified`, the file's view within its budgets | T7 | TUI-6 |
| 38 | `whitelist list --verify` | 5 whitelist | `V` | the password, then the checked list | T7 | TUI-6 |
| 39 | `whitelist add` (address, `--chains`, `*`) | 5 whitelist | `a` | the form, the summary, the password, the outcome | T7 | TUI-6 |
| 40 | `whitelist add --contract --selectors` | 5 whitelist | `a`, kind contract | as 39 | T24 | TUI-6 |
| 41 | `whitelist remove` | 5 whitelist | `d` | as 39 | T23 | TUI-6 |
| 42 | `whitelist fee-cap` | 5 whitelist | `f` | as 39 | T23 | TUI-6 |
| 43 | `whitelist allow-message-signing on\|off` | 5 whitelist | `m` | as 39 | T25 | TUI-6 |
| 44 | `whitelist reset` | 5 whitelist | `R` | as 39, the cost spelled out in the summary | T23 | TUI-6 |
| 45 | `--unlock <dur>` with a change | 5 whitelist | the duration field | a new session after the change | T7 | TUI-6 |
| 46 | `tui --chain`, `tui --warm` | start | the flags | the first chain; the warm-up's duration | T8, T2 | TUI-1b; TUI-2 (T2), TUI-3 (T8) |
| 47 | `--key <name>` | 1 keys | Enter | the key open in the TUI; not a global switch | T2 | TUI-2 |
| 48 | `--version`, `--help` | action menu | `?` | the menu is the help; its last line is `zwallet --version`'s | T12 | TUI-3 |
| 49 | Tron: the address | 4 receive | per 05-tron.md | as 12, with the Tron address and its own trust words | T38 | TUI-12 |
| 50 | Tron: TRX and TRC20 USDT balance | 2 wallet | `c`, tron | as 17 | T39 | TUI-12 |
| 51 | Tron: transfer | 3 send | as 21 | as 21; see the payee note below | T40 | TUI-12 |
| 52 | Tron: transaction lookup | 2 wallet | `/` | as 31 | T41 | TUI-12 |
| 53 | `transfer --to 0x…` (a typed payee) | none | none | **CLI-only, security** (§Non-goals) | T6b (it cannot be entered) | TUI-5 (the refusal) |
| 54 | `transfer --force` | none | none | **CLI-only, funds safety**: the zero address burns what it gets; `check_recipient` refuses it without `--force` (chains/evm/rpc.rs:1052-1066), and the whitelist does not refuse a zero-address entry (`check_shapes` checks the shape only, core/whitelist.rs:689-700). The TUI shows that refusal and offers no override. A token contract as payee is refused by the daemon anyway (README.md:288-293) | none | none |
| 55 | `transfer --token <not in the registry>` | 6 tools, token (balance only) | none | **not a function**: the daemon refuses a `transfer` of a token outside the registry (README.md:288-293); the TUI shows its balance and offers no send | T37 | TUI-11 (the balance; no send) |
| 56 | `invoke --to <a contract not listed>` | none | none | **not a function**: the daemon refuses any other call outside a contract entry (README.md:294-301); the TUI offers listed contracts only | T34 | TUI-11 (the refusal) |
| 57 | `--json`, `--password-stdin`, `--no-qr` | none | none | **not applicable**: the TUI is the human's surface (`tui --json` is refused, README.md:85), it has its own password field (§Password handling), and it draws a QR only on `Q` | none | none |
| 58 | `daemon` (hidden) | none | none | **not applicable**: `unlock` starts it | none | none |
| 59 | `sign-message` with a message longer than 512 characters, or with a character outside printable ASCII (a multi-line SIWE message) | none | none | **CLI-only, security**: the review must show the whole message the key signs, in one bounded block (rules 4 and 5: one source, at most 512 characters, at most 7 rows); a newline or a non-ASCII character would show as a `\u{…}` escape the human cannot read back as the bytes signed. Row 36 covers every other message | T36 (refused at the field) | TUI-11 (the refusal) |

**TUI functions with no CLI command** (from [`v0-2-tui/ssot.md`](v0-2-tui/ssot.md), 2026-10-20):
the history of the last 10 transactions (T3, T3b), the transaction detail with its fee,
confirmations and token movements (T4), and `dropped/replaced` (T6c).

**Tron payees.** A whitelist entry's address is `0x` and 40 hex digits (`check_shapes`,
core/whitelist.rs:700); a Tron address is base58check. Before rows 50–52 can send, 05-tron.md
decides how a Tron payee is listed and signed for (its own entry shape, or the 21-byte hex form),
and the TUI's address fields (rule 3) gain that alphabet. Inputs, outputs and errors of rows
49–52 come from 05-tron.md; the screens are the EVM ones with Tron's values.

**Count.** 59 rows: 52 reachable from the TUI (rows 49–52 once 05-tron.md and its library land),
3 CLI-only with their security reason (53, 54, 59), 2 that the daemon never signs (55, 56), 2
not applicable (57, 58).

## Open trade-offs

Two re-decided non-goals have no safe default. Each is stated with its options and a
recommendation; the dispatcher takes them to review and the owner decides. Neither blocks a
unit: TUI-4 ships Q1 and TUI-10 ships B1, the one-line pointers to the CLI; Q3 or B2 lands as a
delta of its unit (T18, T32) once the owner picks it.

### Q. The QR code on 4 receive

Facts: the CLI ends human output with a QR of an address verified by the password or by the
session (README.md:250-262; `with_qr`, commands.rs:620). That code is the CLI's one write past its
filter (core/terminal.rs:6-7). It is version 3: 29 modules and a quiet zone of 4, 37 columns and
19 lines of half blocks (src/qr.rs:119-122), which rule 1 would turn into `?`. Rule 5 says the TUI
draws no QR.

| Option | What | Security | Fits | Cost |
|---|---|---|---|---|
| Q1 | No QR in the TUI; 4 receive says `zwallet address shows it as a QR code` | nothing new | any size | the function stays CLI-only |
| Q2 | The CLI's own bytes: `qr::render` written to the terminal past `FilteredBackend`, on a full-screen overlay, for an address verified by the password or the session | one more unfiltered write in the TUI, the CLI's own; its only input is an address the TUI derived or pinned | 80x24 (37x19 and a caption) | ratatui's buffer holds blanks there, so leaving the overlay redraws the whole frame (`Terminal::clear`) |
| Q3 | zwallet's QR in ASCII: each module is two spaces in one of two new style constants, `QR_DARK` (black background) and `QR_LIGHT` (white background), on a full-screen overlay | rule 1 stays without an exception (every cell is a space); rule 2 gains two constants that only the QR widget draws (a test asserts no other cell has them) | 120x40 or larger (74x37 cells, a caption, a hint: 39 rows); at 80x24 a line says so and names `zwallet address` | whether a phone reads two cells per module depends on the font `[unverified]` |
| Q4 | Suspend: leave the alternate screen, print `zwallet address`'s own output, return on Enter | the CLI's own path, unchanged | any size | `Guard` takes the terminal once per process (§Password handling) and would need a suspend and a resume (src/tui/term.rs); the code stays in the scrollback |

**Recommended: Q3.** Rule 1 holds with no exception, and every review round of TUI-1a and TUI-1b
leaned on it; the cost is the terminal size only, and at 80x24 the CLI's code is one command
away. If the owner wants the code at 80x24: Q2.

### B. The recovery phrase in the TUI

Facts: `backup` prints the phrase only with stdin and stdout on an interactive terminal, after
the password (commands.rs:1970-1995; README.md:839-841). zwallet's `CLAUDE.md` names `zwallet
backup` as the one exception to "never print secrets". A same-user process can read the human's
terminal (README.md:863-865), tmux's `capture-pane` included, and `zwallet backup` in a tmux pane
stays in its scrollback. The TUI draws on the alternate screen, which a terminal drops when the
program leaves it `[unverified for every terminal and multiplexer]`.

| Option | What | Security | Cost |
|---|---|---|---|
| B1 | Keep it CLI-only: after `create` and on 1 keys the TUI says `run zwallet backup in a shell to see the recovery phrase` | nothing new | the function stays CLI-only |
| B2 | 1 keys `B`: the password typed for it (also when warm); the words on the alternate screen only; hidden on any key and after 60 s; the line `anyone who can read this terminal can read these words now` | the same exposure as `zwallet backup` (the same terminal, the same password), less persistence (no scrollback); a second code path that holds the phrase | zwallet's `CLAUDE.md` must name this screen beside `zwallet backup`: a rule change only the owner makes (not part of this spec) |

For either: acceptance frames and test output hold only a test seed's words, never a real one;
T32 asserts that no frame outside the phrase screen holds a word of it.

**Recommended: B2.** The owner asked for every function, and B2 shows the phrase no more widely
than `zwallet backup` does today. It needs the owner's yes because it widens the one exception of
the secrets rule.

## Screens and keys

Mockups of every screen: [`03-tui-screens.md`](03-tui-screens.md). Research behind the model:
[`research/tui-ux.md`](research/tui-ux.md).

**Seven tabs.** Row 1 is the tab line (it replaces today's title line, `zwallet tui | keys`):

```
zwallet [1 keys] 2 wallet  3 send  4 receive  5 whitelist  6 tools  7 session
```

| Tab | What it holds | CLI it covers |
|---|---|---|
| 1 keys | the key list; open a key (warm-up), make active, verify every address, repair the cache, new key, the first-run create, the recovery phrase | `list`, `list --verify`, `change`, `create`, `address --verify`, `backup`, `unlock` |
| 2 wallet (home) | balances on the TUI's chain (Enter on one sends it), every network (`a`), the last 10 transactions (Enter: the detail), `/` a hash | `balance`, `tx` |
| 3 send | asset, payee from the checked list, amount, review, confirm, status; speed up, cancel, retry | `transfer` and its flags |
| 4 receive | the open key's address and trust, every family's address, the QR (trade-off Q) | `address`, `addresses` |
| 5 whitelist | the list, its policy, `V`; add, remove, fee cap, message signing, reset | `whitelist …` |
| 6 tools | call, invoke, sign a message, look a hash up, a token by contract | `call`, `invoke`, `sign-message`, `tx`, `balance --token` |
| 7 session | lock state in full, unlock for a typed duration, lock, the chains | `status`, `unlock`, `lock`, `chains` |

```
            +-- 1 keys --Enter (password if locked)--> warm-up --> 2 wallet
            |     s make active · n new key · V verify · A repair · B phrase
 tab line --+-- 2 wallet --Enter on a balance--> 3 send (asset set)
 1..7 [ ]   |     Enter on a row --> transaction --s/x on a pending send--> 3 send (replacement)
            |     a every network · / hash --> transaction
            +-- 3 send: asset -> payee -> amount -> review -> type nickname -> status
            +-- 4 receive: address, trust, Q (trade-off Q)
            +-- 5 whitelist: a add · d remove · f fee cap · m messages · R reset · V check
            |     every change: summary -> password -> outcome (stops every daemon)
            +-- 6 tools: call · invoke (review -> type nickname) · sign (review -> type sign) · tx · token
            +-- 7 session: u unlock for a duration · L lock · the chains
 anywhere:  ? or : action menu · c chain · r refresh · L lock · Esc back · q quit
```

**How a first-time user finds every function.**
1. **The action menu** (`?` or `:`, anywhere outside a text field): one list of every action of
   the TUI. Each row has its key, what it does, the CLI command it matches (the alias, so
   `:whitelist add` or `:backup` finds it, as k9s finds a view by its name) and its tab. Typing
   filters; Enter runs the selected action, which opens its tab, form or popup (yazi's help
   runs what is selected; lazygit's `?` menu filters with `/`). An action that cannot run now
   stays listed with the reason (`needs: a warm-up`, `needs: V`, `needs: a pending send`),
   and one that a later unit brings says `in a later version`. The menu's last line is
   `zwallet --version`'s. It is the help screen: there is no other.
2. **The hint line** (row 22) names the keys of the current screen and ends in `? actions`.
3. **The tab line** shows every tab with its number on every frame.

**Keys.** Global, outside a text field: `1`–`7` tabs, `[` `]` previous and next tab, `?` and
`:` the menu, `c` the chain picker, `r` read everything again, `L` lock every zwallet daemon (the
panic button: no confirm, as today), `V` the password check (§Password check: the open key's,
the active key's and on 1 keys every key's address, and the list), `Esc` back (a popup closes, a
sub-screen returns to its tab), `q` or Ctrl-C quit. Movement: `j`/`k` or the arrows, Enter, Tab
between panes or fields. Screen keys are lowercase; an uppercase key carries weight: it changes
state outside the TUI (`L`, `R` reset, `A` repair) or asks for the password or shows a secret or a
code (`V`, `B`, `Q`).

**Text fields.** While a field has the focus every key goes to it (within its alphabet, rule 3)
except Enter, Esc, Tab, the arrows and Ctrl-C; so `q`, `?` and the digits are typed, not acted on.

**Confirms, by what the action does.**
- Signs or sends something: type the name of what it goes to: the payee's nickname (send), the
  contract's nickname (invoke), `cancel` (a cancel at nonce N), `sign` (a message). Enter takes it
  only when it matches.
- Needs the password: the warm-up, unlock for a duration, `V`, `A`, a whitelist change, `n`, the
  first-run create (typed twice), `B`. The password field is the one of §Password handling.
- Changes the agent's default (`s` make active): `y` or `n`.
- `L`: none (the panic button). `r`, tabs, the chain picker: none (nothing changes outside the
  TUI).

**Chain.** `c` opens the chain picker (`zwallet chains`: key, name, symbol, chain id, RPC host);
the pick is the TUI's own and reloads every pane. The default is `--chain <key>` when given,
otherwise ethereum (T8). Nothing is stored.

- **Status bar:** two lines at the bottom. Line 1: chain · key · short address · address trust
  (§Address trust). Line 2: **`signing (agent too) until 14:32Z`** or `locked` · RPC host
  (rule 3). The expiry is drawn by zwallet from the RFC 3339 time the daemon reports
  (`core::time::parse_rfc3339_utc`), in UTC, with the date in front when it is a day or more
  away (`2027-10-28 14:32Z`); one that does not parse is shown as it came (escaped, at most 64),
  cut to 17 with `...` in the bar. The warm-up opens signing for every client of the daemon,
  including the agent, and the bar says so. The lock state is read again every 5 s and on `r`,
  and is said once, on line 2. After `L` the bar says `checking...` until a fresh status lands;
  a key or status read started before the lock finished is dropped. Each line fits its width
  with every value whole or cut with `...`, never at the edge. On line 1 the trust words of the
  state come first and the key name gives way: it is cut with `...` to what the chain, the short
  address and those words leave, at most 32 and at least 8 (5 characters and `...`). An
  impostor's `NOT this build's key daemon: ` keeps room for 16 characters of its warning before
  the name gets more; the rest of the warning is cut with `...`. Only words that cannot fit beside
  the narrowest name are cut, with `...` too; at 80 columns none are (the longest, the 40 of
  `not verified (no session check on Linux)`, fit beside 8 with the widest chain). In the key
  list the name is cut to 27, or less when the index is wider than three digits, so the address
  after it is whole; on line 2 the expiry is cut to 17 and a status error to 37, and the RPC host
  is at most 32. `q` while
  `L` is stopping daemons waits for it (`locking... (quits when done)`); a stop that failed is
  printed after the terminal is given back.
- **Review screen:**
  - payee nickname and full address, asset and amount;
  - the fee estimate and the fee cap;
  - the whitelist verdict, the pending guard and the duplicate guard (02 §Send safety).
  - A guard failure shows its message. An override (allow-pending `p`, allow-duplicate `d`) is a
    separate key press, with the warning on screen. `g` sets a fee ceiling in gwei
    (`--max-fee-gwei`) and runs the review again.
- **Terminal:** `zwallet tui` needs stdin on a terminal (raw mode is set on stdin, and the panic
  hook restores stdin's modes) and stdout on one; otherwise it refuses before taking either.
- **Size:** below 80 columns or 24 rows the TUI draws one line, `terminal too small (80x24
  needed)`, and nothing else. At 80x24: the tab line, the pane (bordered with `+ - |`, 78 columns
  inside), a key-hint line, and the two bar lines. Popups are drawn over the pane (`Clear`, then
  an ASCII-bordered block). At 120x40 the wallet tab puts its two panes side by side.

## Display integrity in the TUI

The CLI writes every human byte through one filter, `core::terminal::write_human`
(core/terminal.rs:44). ratatui instead puts characters into cells and writes the cells to the
terminal itself. So the TUI gets a write point of its own (rule 1) and states the CLI's other
rules for its cells. The threat is the one the README states: a process of this user controls
some of the text zwallet shows, and must never make zwallet's own screen show its address as
verified or fundable: no QR code that holds an address, no forged "verified" mark, no terminal
control.

**1. One write point, enforced at run time.** A cell may hold exactly one character of printable
ASCII (`0x20`–`0x7E`) or `₮`: `TERMINAL_CHARS` (core/terminal.rs:20) without the newline, which a
cell never holds, as `terminal_ok` (:24) tells. One `Backend` wrapper, `FilteredBackend` in
`src/tui/term.rs`, is the only way the TUI writes to the terminal. Before each draw it replaces
every cell whose symbol is not exactly one `terminal_ok` character with `?`, and counts the
replacements. A replaced cell is one column wide: a glyph two columns wide becomes `?`, and the
wrapper itself writes a space into the column the glyph covered, so the result does not depend on
whether ratatui's diff sends that column. Those spaces stop at the screen's width from the last
size read through the wrapper (with a fullscreen or inline viewport, ratatui's `Terminal` reads one
when it is made and before every frame; a fixed viewport reads neither, and the TUI's is
fullscreen); a draw never asks the terminal, which fails the frame where none answers (no tty, no
`TERM`). Before any size has been read it writes no space: no column is known to be on the
screen, and one past the right edge would land on the last column. Tests draw through the same
wrapper over ratatui's `TestBackend` and assert zero replacements on every frame: the wrapper is
the last filter, as `write_human` is, and escaping where text comes in (rule 3) stays mandatory.
The TUI's own chrome keeps to the set too: borders are `+`, `-` and `|`, the selection marker is
`>`, the password mask is `*`, and busy states end in `...` (`checking...`, never `…`).

What rule 1 rules out is terminal control **from data**: no byte sequence that came from a cell's
content (an ESC, a CSI or OSC sequence, a BEL) ever reaches the terminal. The backend's own
control sequences, which move the cursor and set the styles of rule 2, do reach it: they come
only from ratatui and crossterm, never from a cell's content, as the CLI's only unfiltered bytes
are its own QR code's (core/terminal.rs:6-7).

**2. Colour and style come from code only.** Every style the TUI uses is one of a few constants
in `src/tui/style.rs`. No data chooses a colour, a reverse video or a background. A test asserts
that every cell's style in every frame is one of those constants.

**3. Every string from outside is made inert and bounded where it comes in, with the CLI's own
functions and budgets.** "From outside" means text zwallet does not write: a file in the home that
any process of this user can edit, a reply on `d.sock`, an RPC endpoint, the explorer, the
environment, and what the human types.

| Source | What the TUI shows from it | Check and budget (the CLI's own where there is one) |
|---|---|---|
| `wallet.json` key names and cached addresses | T1 rows, the status bar | names (and `active`) pass `validate_key_name` when wallet.json is parsed (core/store.rs:568-576), else the file is refused; a cached value that is not an address is an error that shows it escaped, at most 64 (`cached_address`, commands.rs:251; `VALUE_SHOWN`, :195) |
| `d.sock`: an address | T1 rows, the status bar | `reply_address` (commands.rs:267): an address's shape, else an error that shows it escaped, at most 64 |
| `d.sock`: the session's expiry | the status bar | `shown`, at most 64 (commands.rs:1827) |
| `d.sock`: the session's whitelist view | T7, as information only | `inert_whitelist` (commands.rs:208); the TUI draws its `gen` and the count of its `entries`, never an entry string, never the word "verified" (§Password check) |
| `d.sock`: an error reply | any error line | `relayed_error` (core/protocol.rs:329): the message and its kept details share 512; from another process, named, details dropped |
| `d.sock`: who answered | the status bar's warning | `PeerCheck::warning` (core/protocol.rs:1042); the executable path at most 200 (`EXE_SHOWN`, :1052) |
| `whitelist.json`, `whitelist.next.json` | T6 picker, T7, T4 nicknames | shape-checked at parse (`check_shapes`, core/whitelist.rs:689: nickname `a-z 0-9 _ -`, `0x` hex address and selectors, chain-key shape, decimal caps), else not a list; the parser's message quotes at most 80 (`clip`, :450); names and fee caps from an unchecked file share one 512 budget (`FileTextBudget`, commands/whitelist.rs:899); one field at most 64 (`SHOWN_CHARS`, :1236); a password-checked list up to 100 entries, an unchecked one within one 512 budget (`list_text`, :1459-1520); chains or selectors at most 16 per entry (`ITEMS_SHOWN`, :1244). An address or a nickname that a payee picker, a review or a T4 label shows comes only from the password check |
| the send journal | T3 rows, T4 local record, T6 guards | each field in the shape zwallet writes it, else `<invalid>` (`EntryText::of`, chains/mod.rs:756); at most 512 per message (`JOURNAL_TEXT_MAX`, :692) |
| an RPC endpoint's error | T3 network line, T4, T6 | scrubbed of the URL's key (`Net::redact`, chains/mod.rs:282), escaped and cut to 512 where the error is built (`failed`, chains/evm/rpc.rs:109-140); a network's line cut to 72 (`brief`, commands.rs:1143, :1355) |
| `ZWALLET_RPC_<CHAIN>` (the environment) | the status bar's RPC host | the host (and port) of the origin `redact_url` keeps (`url_origin`, chains/mod.rs:476: key-like host labels redacted), without the scheme and without anything after the origin; at most 32, cut with `...`; a value that is not a URL with a host `<redacted>` |
| the explorer | T3 rows | the shape check in §History; no free text from it is ever shown |
| what the human types | input fields | each field takes only its own alphabet at the key event: an amount `0-9` and one `.`; a nickname `a-z 0-9 _ -`, at most 32 (`validate_nickname`, core/whitelist.rs:455); an address `0x` then `0-9 a-f A-F`, at most 42; a duration `0-9` and one of `s m h d w y`; the password any byte, shown only as `*`. Any other key changes nothing and nothing else is echoed. Added 2026-10-28: a key name as `create` takes it (`validate_key_name`, core/store.rs:176); a tx hash `0x` and 64 hex digits; a selector `0x` and 8 hex digits; calldata `0x` and hex digits, at most 4096; a function signature and its arguments printable ASCII, at most 256 and 512; a message to sign printable ASCII, at most 512 (a paste holding anything else is refused whole, §Coverage matrix row 59); a word count one of `12 15 18 21 24`; a bump factor `0-9` and one `.`; a Tron address per 05-tron.md |
| `call`'s result | 6 tools | the CLI's `render_human` of the decoded result and the returned bytes as hex cut to 512 (commands.rs:1424-1435); drawn as a result view, rule 4 |
| a token's own `symbol()` and `decimals()` | 6 tools, token | escaped, the symbol at most 32 (`erc20_meta`, chains/evm/rpc.rs:209; README.md:212-213) |
| the home path (`ZWALLET_HOME` or `HOME`) | 7 session | escaped, at most 64, cut with `...` |
| `status`'s pids | 7 session | numbers only; at most 8 of another home's pids, then how many more |
| `address --verify`'s `repaired` list | 1 keys after `A` | key names as above; `was` escaped at most 64, `now` derived from the seed (README.md:119-123) |

`core::text::escape` turns `₮` into `\u{20ae}` like every non-ASCII character (core/text.rs:40-50;
its test, :229), so `₮` reaches a cell only from zwallet's own registry constants
(chains/evm/registry.rs:71, chains/evm/rpc.rs:237-240) or from a journal amount in the shape
zwallet writes (chains/mod.rs:739).

The cell rule does not prove the budgets or where a value came from, so each is asserted on its
own: every outside value reaches a widget as `tui::text::Outside { source, text }`, built only by
the function of its source above, and the tests assert, per source, that the drawn text is within
that source's budget and appears only on the line or block of that source.

**4. Placement.** In list panes and the status bar each outside value sits on one line, cut to
the pane's width, never wrapped. Only an error view wraps text: the error detail view (Enter on
an error line), a pane whose whole content is one error (the keys pane when the key list
cannot be read), or, added 2026-10-28, `call`'s result view (one result, nothing else in the
pane):
one source alone at a time, the text already escaped (`escape`, core/text.rs:40-50, which
`ascii_inert`, :69, runs first, turns an outside newline into `\u{a}`, so a newline can never
start a row of its own), at the pane's inner width of at least 78 (the size rule keeps the
terminal at 80 columns or more). 512 characters then fill at most ⌈512 / 78⌉ = 7 rows. Two
outside sources are never wrapped in one block.

**5. Why no outside text can draw a code that holds an address.** A QR code can be drawn with
ASCII characters or with background colours as modules, so excluding block glyphs (rule 1) is not
enough on its own. The defence is the three rules together: styles are zwallet's (rule 2), so data
can draw nothing with colour; placement (rule 4) keeps a source to one line, or to at most 7 rows
in the error view, where the smallest code is 21 rows tall; and every source is bounded (rule 3)
to at most 512 characters, where a code that holds an Ethereum address needs version 2, 25 × 25
= 625 modules (README.md:198-200). The one list drawn with more than 512 characters from a file is
a password-checked whitelist, which only the password holder signed. What is left is what the
README already states: a same-user process can lay out at most 512 characters of ASCII art, and it
can already write to the terminal directly. The TUI itself draws no QR code. Tests: a 512-character
outside error of `#` and spaces in the error view takes at most 7 rows; the style assertion of
rule 2; the budget assertions of rule 3. Whether the TUI draws zwallet's own QR of a verified
address is trade-off Q (§Open trade-offs): Q1 keeps this paragraph as it is; Q3 adds two style
constants that only the QR overlay draws, from an address the TUI derived or pinned, never from
outside text; Q2 adds one write past the filter, the CLI's own (core/terminal.rs:6-7).

**6. Nothing prints while the TUI holds the terminal.** Four places in today's code write to the
terminal from inside the logic (flow.md Fact 3): the `broadcasting <hash>` line of every live send
(chains/mod.rs:1215), `create`'s backup note (commands.rs:752-755), a whitelist summary with
`--password-stdin` (commands/whitelist.rs:264) and the password prompt (core/password.rs:152).
The entry points the TUI calls return that text instead; the TUI shows it in a pane, through
rules 1–4.

**7. The word "verified".** The TUI draws "verified" (or any mark that says so) only for data the
password checked in this TUI session (§Password check), or, for an address only, for a derivation
by this build's key daemon of this home, which the kernel confirmed (`PeerCheck::ThisBuild`, macOS
only), as the CLI marks it today (`Trust::Session`, commands.rs:555-566). Nothing a daemon reports
about the whitelist, and nothing a file says, is ever drawn as verified. On 1 keys a row says
`verified` only for a key whose address the password check of this TUI session derived
(the warm-up's named keys, or every key after `V`), or the active key while the bar says
`verified by session`.

**Addresses a human may copy or pay** (the payee picker, the review, the add screen, T4's from
and to) are always shown in full EIP-55, rechecksummed by zwallet. Only the status bar shortens
the wallet's own address.

## Address trust and session verification

The TUI shows an address's trust as the CLI's `verified` and `verifiedBy` say it
(README.md:92-149; `Trust`, commands.rs:371-411), on the status bar, for the key open in the TUI:

| Bar | When | Code |
|---|---|---|
| `verified by password` | the password check of this TUI session derived this key's address from the seed (§Password check) | `derive_checked` (commands.rs:647), as `list --verify` derives it |
| `verified by session` | unlocked, no password check yet, and every answer for the address came from this build's key daemon of this home (macOS only) | `Trust::Session` (commands.rs:555-566), `check_peer` (core/protocol.rs:1078) |
| `not verified` | locked: the cache (line 2 says `locked`; the lock state is said once, T1) | `Trust::Locked` |
| `not verified` | unlocked, but no check passed: a cached address, or Linux, where the daemon's executable is hidden and the session check never passes (core/protocol.rs:1170, :745-751). On Linux the bar adds `(no session check on Linux)` | `Trust::Unverified(None)` |
| `NOT this build's key daemon: pid N ...; run zwallet lock and unlock again` | another process answered on `d.sock` | `Trust::Unverified(Some(warning))`, `PeerCheck::warning` (core/protocol.rs:1042) |

The last line replaces the trust state, cut to the bar's width; Enter on it shows the whole
warning (within its own budget, §Display integrity rule 4). A cache that the daemon's answer
contradicts stops the pane with the CLI's `VALIDATION_ERROR` (commands.rs:520-536): fund neither
address.

**Pinned for the session.** The addresses the password check derived (the open key's, and the
active key's) are kept for the life of the TUI. If a later session read (`Daemon::address`,
readiness, a status refresh) gives another address for either key, the TUI stops that pane with
the mismatch error, worded as `check_cache` words one (commands.rs:285-296), and never takes the
new address silently.

## Password check

When the human types the password in this TUI session (the warm-up, a whitelist change, or `V`),
the TUI also opens `wallet.json` with it, as `list --verify` and `whitelist list --verify` do in
the CLI process (flow.md §Password check). It costs one more scrypt run, the cost of `list
--verify`, so it runs off the UI thread (`tokio::task::spawn_blocking`, which the `rt` feature
the CLI already has provides, Cargo.toml:29); meanwhile the pane says `checking...` and the frame
keeps drawing. Two entry points do it, in one blocking task:

1. `wallet::open_with_password(home, password, keys)`: `store::unlock` (core/store.rs:643)
   decrypts the seed and checks every cached address against it; a modified cache is the CLI's
   `VALIDATION_ERROR`, which says to run `zwallet address --verify`. Then the named keys' (the
   open key's and the active key's) addresses are derived from the seed (`derive_checked`,
   commands.rs:647): `verified by password`, pinned (§Address trust). The seed is dropped before
   it returns. It returns `Opened { addresses, anchor, salted }` and nothing else: `anchor` is the
   sealed `WhitelistAnchor` (public key, generation, digest; core/store.rs:419), read through
   `sealed_whitelist` (:255), and `salted` says whether a salt is sealed with it. The salt itself
   (the private field of `SealedWhitelist`, core/store.rs:461-467) never leaves the blocking task
   and is never in the checked list.
2. `wallet::check_list(home, anchor, salted)`: an anchor sealed without a salt (zwallet 0.1.0)
   gets `list --verify`'s message (commands/whitelist.rs:1353-1360). Both list files are read once
   (`ListFiles::read`, core/whitelist.rs:987) and the list the seal names is picked from those
   bytes (`pick_base_from`, :1097, which verifies the signature, generation and digest, `verify`,
   :766), as `verified_view` does (commands/whitelist.rs:1373-1403). Checking a signature needs
   only the sealed public key; the signing key is derived (password + sealed salt,
   core/whitelist.rs:727) only for an edit.
3. The TUI keeps the result in memory for the TUI session as its **checked list**: the verified
   entries (nickname, address, chains, kind), the list's own policy (`Whitelist.policy`,
   core/whitelist.rs:226-231, `Policy` :158: the fee caps and the message-signing flag; not
   `Unsealed.policy`, the wallet's sealed policy object, core/store.rs:241-244), `gen`, the source
   file, the verified list itself, and the fingerprint of the bytes it verified (`ListFiles::fingerprint`,
   core/whitelist.rs:995: the keccak-256 of each file's bytes, :970-975).

The CLI calls the same two: `list --verify` (commands.rs:827-829) calls `open_with_password` with
every key, and `whitelist list --verify` (commands/whitelist.rs:1345-1362) calls both. `address
--verify` does not move: it repairs a modified cache from the seed (`store::unlock_repairing`,
commands.rs:956-957) where the password check refuses one, and it reads no list file, so it
behaves exactly as today.

A cache modified before the warm-up fails the warm-up itself: the daemon decrypts with
`store::unlock` too (daemon/mod.rs:311), so no session starts. A cache modified after the session
started and before the password check (a process of this user racing the TUI) is caught by step
1: the session stays, the pane shows the error, and no list is checked and no address is pinned.

What the checked list is used for, on macOS and Linux alike:
- the payee picker offers its address entries that apply on this chain (`Entry::covers_chain`,
  core/whitelist.rs:141), and nothing else;
- T7's `verified` state is the checked list's;
- T4 labels a from or to address with a nickname only from it.

The TUI checks that the list files are still the bytes it verified (`whitelist::check_unchanged`,
core/whitelist.rs:1022) at every point where it is about to use the checked list: when a pane that
shows it opens or refreshes (T7, T4's labels), when the payee picker opens, before the review is
shown, and at confirm. Any difference drops the checked list: the screen goes back to `not
verified` and offers nothing.

**Without a password in this session** (the TUI started while the wallet was already unlocked),
the transfer screen and T7 show the list as `not verified` and offer one key press, `V`: type the
password to check the list. `V` verifies only: it starts no session, stops none, and changes no
file.

**The session's whitelist view** (the daemon's `{gen, verified, entries}`, daemon/mod.rs:123-138)
is shown in T7 as information: "the running session reports gen N, M entries; this TUI did not
check it". It is never the source of an address and never drawn as verified. Its source cannot be
trusted: `protocol::status` is a plain request with no peer check (core/protocol.rs:434-436), and
`status` accepts the reply of any pid the process table lists for the home by argv
(commands.rs:1884-1903), which a process of this user can choose.

## Whitelist edits

Every change in the TUI runs the edit protocol of 04-whitelist.md §Edits, through the two entry
points flow.md §T7 names, and nothing else:

1. `wallet::whitelist_plan(home, change, unlock)`: checks the change as the list will, runs
   `unlock`'s own checks and pins the daemon binary when the human chose a duration, reads both
   list files once, and returns the summary of what the password authorizes with the files'
   fingerprint (step 0; commands/whitelist.rs:240-262). The summary is drawn above the password
   field. It is bounded already (`FileTextBudget`, commands/whitelist.rs:899) and passes the cell
   rules. Nothing is stopped yet.
2. The password is typed for this change, even while warm: the TUI keeps no password once it has
   been used (§Password handling).
3. `wallet::whitelist_apply(plan, password)` (steps 1–5; commands/whitelist.rs:269-318): the
   password is checked against `wallet.json` (a wrong one stops nothing); the whitelist key is
   derived from the password and the salt sealed in `wallet.json` (core/whitelist.rs:727,
   core/store.rs:391; there is no `whitelist.key`, 04-whitelist.md:104); the files must still be
   the bytes shown; **every zwallet daemon of this user is stopped** (all homes, the `zwallet
   lock` path, core/protocol.rs:1783); the edit runs under `w.lock`; and the wallet is **left
   locked**, unless the human typed a duration with the change (`--unlock <dur>`), when a new
   session runs for that duration from now. No duration or expiry is taken from the session that
   was running.
4. With the same password the TUI runs the password check again (§Password check), so the
   checked list is the new one, then clears the password.
5. The TUI shows the outcome (generation, entries, stopped daemons, warning, session) and every
   pane reloads: the old session has ended.

## History (re-measured 2026-10-27)

Sources:
1. **The send journal.** This is a *local claim*. Every row from it is looked up on chain (`tx`)
   before it shows a status.
2. **A keyless explorer**, where one answers a non-browser client: `eth.blockscout.com`,
   `explorer.optimism.io` and `eth-sepolia.blockscout.com`. Base, arbitrum and polygon Blockscout
   return a Cloudflare challenge (HTTP 403); bsc and avalanche have no Blockscout instance.

**Reachability, measured 2026-10-27 11:53:24Z** with `curl 8.7.1`, no key, `GET
/api/v2/addresses/0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045/transactions`; rechecked by 11:54:29Z
with no `User-Agent` header, as reqwest 0.13.5 sends by default (its client builder sets only
`Accept`: the crate's own source, `reqwest-0.13.5/src/async_impl/client.rs:284-285` in the cargo
registry, not a zwallet file). Routescan's v2 API (`api.routescan.io/v2/network/mainnet/evm/<chain
id>/address/<a>/transactions`) was probed in the same minutes:

| Chain | Host | HTTP | Content-Type | Notes |
|---|---|---|---|---|
| ethereum | `eth.blockscout.com` | 200 | `application/json` | 50 items, 523 kB; `token-transfers` 200; no-UA 200 |
| base | `base.blockscout.com` | 403 | `text/html` | Cloudflare "Just a moment..."; `token-transfers` 403; no-UA 403. Routescan 8453: 400 `NO_BLOCKCHAINS_FOR_PARAMS` |
| arbitrum | `arbitrum.blockscout.com` | 403 | `text/html` | Cloudflare challenge; `token-transfers` 403. Routescan 42161: 400 `NO_BLOCKCHAINS_FOR_PARAMS` |
| optimism | `optimism.blockscout.com` | 301 | `text/html` | redirects to `explorer.optimism.io` |
| optimism | `explorer.optimism.io` | 200 | `application/json` | 50 items; no-UA 200 |
| polygon | `polygon.blockscout.com` | 403 | `text/html` | Cloudflare challenge; `token-transfers` 403. Routescan 137: 400 `BLOCKCHAIN_NOTFOUND` |
| bsc | `bsc.blockscout.com` | 404 | `text/plain` | "default backend - 404": no instance. Routescan 56: 400; its Etherscan-style path says `chain not supported` |
| avalanche | `avalanche.blockscout.com` | 404 | `text/plain` | no instance. Routescan 43114: 200 JSON (no-UA 200), another format, not used in v0.2 |
| sepolia | `eth-sepolia.blockscout.com` | 200 | `application/json` | 50 items; `token-transfers` 200; no-UA 200 |

Before TUI-7 merges the table is measured again, Routescan's other paths for base, arbitrum and
polygon included, and that data decides whether one more parser opens those chains
([`v0-2-tui/loop.md`](v0-2-tui/loop.md) §Entry conditions). Until then the coverage note per chain
follows this table: ethereum, optimism and sepolia show the explorer and the journal; base,
arbitrum, polygon, bsc and avalanche show "this machine's sends only; incoming transfers not
indexed".

ERC-20 log scanning was dropped. Public nodes only serve about 5k blocks of logs (roughly 20 min on
arbitrum, about 3 h on base). Those logs are also where address-poisoning `Transfer(you,
lookalike, 0)` rows show up. (T4 reads the logs of one receipt, the selected transaction's: that
is no scan.)

**The explorer is an outside source.** Its operator, and anyone who can make it index something
(a token contract names itself; anyone can send a 0-value transfer or mint a token to the
address), writes what it returns. So it is read like `d.sock` or an RPC endpoint:

- **Requests.** `GET <base>/api/v2/addresses/<address>/transactions` and `GET
  <base>/api/v2/addresses/<address>/token-transfers?type=ERC-20`, the first page only, no key, no
  cookie, through the `reqwest` alloy already brings (flow.md Fact 4). `<base>` is fixed per chain
  in code (the table above): no flag, environment variable or file changes it (ssot.md:70 deleted
  the history URL override; no settings file). The body is read up to 1 MiB (the size the daemon
  socket allows a reply, `MAX_MESSAGE`, core/protocol.rs:49), within 8 s per chain (as `balance`
  waits per network, `NETWORK_TIMEOUT`, commands.rs:1138).
- **Unavailable.** Anything but HTTP 200 with a JSON object holding an `items` array makes the
  explorer unavailable for that refresh. The note then says `<chain>: explorer unavailable (HTTP
  <code>)` or `(not JSON)`: zwallet's own text and the status code, never a byte of the body (T3b).
- **Fields kept, each with its shape; an item with any field out of shape is dropped whole.**
  - a transaction: `hash` (`0x` and 64 hex digits); `block_number` (at most 20 decimal digits, or
    `null` while pending); `position` (at most 10 digits); `nonce` (at most 20 digits);
    `from.hash` and `to.hash` (`0x` and 40 hex digits; `to` may be `null`, shown as "contract
    creation"); `value` and `fee.value` (at most 78 digits); `status` (`ok`, `error` or `null`);
    `timestamp` (RFC 3339, parsed and written again by zwallet, as the journal's time is,
    chains/mod.rs:769-772).
  - a token transfer: `transaction_hash`, `block_number`, `log_index`, `from.hash`, `to.hash` in
    the shapes above; `token.address_hash` (`0x` and 40 hex digits), which must be a registry
    token on this chain (`registry::lookup`, chains/evm/registry.rs:109); `total.value` (at most
    78 digits). Its symbol and decimals come from the registry, never from the explorer.
  - Every kept address is parsed and shown rechecksummed by zwallet. An item whose `from` and
    `to` are both other than the wallet's address is dropped (it is not about this wallet).
  - Dropped, as the address-poisoning pattern: a token transfer of a token outside the registry,
    of an NFT, or of 0; and a transaction of `value` 0 sent by another address to the wallet (it
    carries nothing; a token it moved still shows through its own token-transfer item).
- **Never read into anything shown:** every name or label the explorer puts on an address
  (`name`, `ens_domain_name`, `public_tags`, `private_tags`, `watchlist_names`, `metadata`,
  `is_scam`, `reputation`, `implementations`), `method`, `decoded_input`, `raw_input`,
  `revert_reason`, `transaction_tag`, `token.name`, `token.symbol`, `token.decimals`,
  `token.icon_url`, `exchange_rate`, `historic_exchange_rate`, `confirmations` (the TUI reads its
  own from the node), and every other field. All of these were in the 2026-10-27 bodies.
- **Counted, not shown.** The coverage note says how many explorer items were dropped ("3 explorer
  rows not shown: not in the shape zwallet reads").
- **Rows are claims.** An explorer row is labelled `explorer`. T4 reads the transaction and its
  receipt from the node; when the node does not know it, the detail says "the explorer lists it;
  this node does not know it". No explorer value ever fills an address field (T7 add never
  pre-fills from history).

**Merge contract:**
- Rows are keyed by `(chainId, txHash)` and de-duplicated. One row can carry several token
  movements, and lists every source that reported it.
- Order: pending first, then block number descending, then by position in the block.
- The home view shows the first 10.
- A coverage note is always visible, for example "base: this machine's sends only; incoming
  transfers not indexed".

**T4's token movements come from the node.** For the selected transaction, `rpc::tx_detail` reads
its receipt and decodes every log that is an ERC-20 `Transfer` (three topics, the first
`keccak256("Transfer(address,address,uint256)")`, 32 bytes of data) emitted by a registry token on
this chain, whose from or to is the wallet. That covers a transfer routed through another
contract (a router, a batch distributor), where the transaction's own `to` is that contract. The
detail shows the transaction's from and to on their own lines, then one line per movement (from,
to, amount with the registry's symbol). A reverted transaction moved no token and says so. A
pending one has no receipt: "token movements unknown until it is mined". A log of a contract
outside the registry, and a movement of amount 0 (the address-poisoning pattern §History drops
too), are counted, not shown.

## Status of a send

States: `pending → mined (success | reverted)`. A tx becomes **`dropped/replaced`** when its nonce
has been mined by a different hash: the account's mined nonce (`eth_getTransactionCount` at
`latest`) is past the tx's nonce and the tx has no receipt. No code tells this today (flow.md §T6:
`lookup_tx` knows only mined, pending and not found); `rpc::tx_state` will. The screen polls every
3 s (as `RECEIPT_POLL`, chains/evm/rpc.rs:28). After 90 s (as `--wait`, commands.rs:36) the TUI's
own send-status state (`src/tui/app.rs`, TUI-5) turns into `unknown — run zwallet tx`; it does not
use `record_receipt`, which reports a receipt that did not come in time as `pending`
(chains/mod.rs:1396-1399). Leaving the screen never cancels anything; the journal has already
recorded the send.

**Review and confirm.** The review runs `prepare` as a dry-run, the duplicate check and the
whitelist check; it takes no send lock and asks the daemon to sign nothing. The whitelist verdict
and the fee cap, at the review and at confirm, are computed on the checked list (the same bytes
its fingerprint holds): `policy::check_tx` (chains/evm/policy.rs:65) on `ListView::Usable` of the
checked list (:41-55), never on whatever whitelist.json says at that moment (the CLI's
`dry_run_check` reads the file, commands/whitelist.rs:980). The review holds the exact
transaction it showed: chainId, from, the payee address (EIP-55, taken from the checked list), the
asset (native, or the registry token's contract), the amount in base units, the calldata, and the
nonce, the most the fee can cost, `sufficientFunds` and the whitelist verdict. The most the fee
can cost is the number the review showed: `max_gas_cost` plus `l1_fee` where the chain has one
(base and optimism; `prepare`, chains/evm/rpc.rs:736-744), held as U256 integers. `prepare`'s
output fields hold them only as formatted strings (`maxGasCost`, `l1Fee`, :797-804), so the
entry point returns the integers too, and the comparison never reads a string.
Confirm checks the list files against the checked list's fingerprint, takes the send lock
(`SendLock`, chains/mod.rs:651), runs `prepare` again (fresh nonce and fees, the payee address
held by the review, never looked up again by nickname), runs the pending and the duplicate guards
again (with the override the human chose at the review, if any), and decodes the new unsigned
transaction (`tx::decode_unsigned`, chains/evm/tx.rs:378: `chain_id`, `nonce`, `to`, `value`,
`input`, `action`, :329-358). It signs only if the transfer intent (chainId, from, payee, asset,
amount, calldata, nonce), `sufficientFunds` and the verdict are exactly equal, and the most the
fee can cost is at most the reviewed one. A higher fee, or any other difference, signs nothing and
shows the new review. The `l1_fee` part is set by the sequencer when the transaction is included,
so the signature cannot bind it (04-whitelist.md:48: the L1 data fee is outside `gasLimit ×
maxFee`): confirm compares the estimate it reads, and a rise after confirm is outside what
zwallet signs. At confirm, a rise large enough to leave the balance short also flips
`sufficientFunds` (which counts `l1_fee`, chains/evm/rpc.rs:742-744), and that exact match catches
it as well.

**Replacements (added 2026-10-28).** Speed up (`s`), cancel (`x`) and retry (`R`, after
`BROADCAST_UNCERTAIN`) are sends at a fixed nonce N with `--replace <hash>`: the CLI's `--nonce`,
`--replace` and `--bump` (README.md:594-622; `replacement`, commands.rs:1521). Speed up keeps the
transfer intent and raises the fees by the bump (default 1.25, from 1.1 to 3, typed on the
review); cancel is 0 of the chain's coin to the key's own address, which the password check
pinned (§Address trust). Each runs the review and confirm above: the review holds nonce N and the
replaced hash, confirm re-prepares with them and signs only on the exact intent. A replacement is
offered only for a send of this machine's journal whose nonce is pending; the pending and
duplicate guards do not apply at a fixed nonce, as in the CLI (README.md:563, :585-586).

**Invoke and sign-message (added 2026-10-28).** An invoke's review holds chainId, from, the
contract (from the checked list), the calldata, the value and the nonce, and shows zwallet's own
decode of the unsigned transaction (`tx::decode_unsigned`, chains/evm/tx.rs:378); confirm follows
the same exact-intent rule. A message's review holds its bytes, the key and the chain, and the
list's message-signing flag from the checked list; confirm sends exactly those bytes, and the
signature is checked against the key's address (`verify_message`, chains/evm/mod.rs:193), as
`sign-message` does (commands.rs:1792-1818).

## Password handling

- The terminal is already in raw mode, so there is no echo, before the field takes input.
- The password has one owner: `PasswordBuf`, a fixed-capacity `Zeroizing` buffer of 1024 bytes
  (the CLI's `MAX_SECRET`, core/password.rs:13), rendered as `*`. The type has no `Debug`,
  `Display` or `Clone`.
- The input thread writes each keystroke and each paste into that buffer itself and posts only
  the new length as a `Msg`. A paste's own copy is zeroized as soon as it is appended, not just
  dropped.
- Password mode is switched on in the same step that handles the Enter opening the field: the
  input thread holds the flag and sets it before it reads the next event, so typeahead after that
  Enter never reaches a `Msg::Key`.
- No `Debug` or logging of any message that carries the password, and no event queue that keeps
  it.
- Before the field opens for a warm-up, `wallet::prepare_unlock` runs `unlock`'s checks and pins
  the daemon binary (commands.rs:1913-1922), so a binary swapped in while the human types is never
  started.
- The buffer is cleared on success, cancel and every failure path. It lives until its last use:
  the session start (if any) and the password check (§Password check), then it is cleared.
- The same field serves every action that asks for the password (§Screens and keys, Confirms).
  The first-run create asks twice, as `create` does (`read_password(.., true)`, commands.rs:702):
  a second `PasswordBuf`, compared, then both cleared. No action keeps the password past its own
  use; the next one asks again.
- A panic hook and normal unwinding both restore the terminal. The panic hook then writes one
  fixed line, "zwallet tui stopped unexpectedly; the terminal was restored", never the panic's
  payload: a constant byte string already in the terminal set (a test checks it), written with
  `write(2)` on fd 2, not through the write filter, which allocates.
- A panic while the TUI holds the terminal ends the process (`_exit(101)`), also one tokio would
  catch (`spawn_blocking`), so no TUI ever goes on after a panic (`src/tui/term.rs`). The hook
  restores the terminal's modes first (`tcsetattr` from the modes the guard recorded), then
  writes the screen's restore sequences on fd 1 and the line on fd 2, each only when `poll(2)`
  says the fd takes it within 100 ms: a full pipe or a stalled reader costs that output, never
  the exit. Right after the modes it arms a 2 s deadline: `alarm(2)` with a SIGALRM handler
  (installed with `sigaction`, SIGALRM unblocked on that thread) that restores the recorded
  modes and calls `_exit(101)`, async-signal-safe calls only; so a write that blocks anyway
  (another writer filled the pipe after `poll`) still ends the process, with the modes back. A
  second panic meanwhile prints nothing and parks inside the hook; a parked thread keeps every
  lock it holds (std's stdout and stderr locks too), which is why the hook's path takes no lock
  and allocates nothing.
- The TUI takes the terminal once per process (`Guard::enter`; every later call is refused, also
  after the first guard dropped or the first call failed), and before it starts any other thread
  or blocking task. So a panic while the terminal is being taken (modes recorded, raw mode on, the
  screen not yet set up) comes on the taking thread, and is handled like one while it is held:
  the recorded modes are restored and the process ends. A panic on another thread then is what
  that call order rules out, and the hook does not make it safe: if it comes before raw mode is
  set, it restores the modes (or finds none recorded), the taking thread then sets raw mode, and
  the process can end raw. The `acquiring-locked` test covers only the other order, raw mode
  first. `_exit` runs no destructors, so a `Zeroizing` buffer such as `PasswordBuf` is not
  cleared on that path: its memory goes with the process.

## Architecture

- `src/wallet.rs` (new): the library entry points [`v0-2-tui/flow.md`](v0-2-tui/flow.md) names.
  The CLI handlers in `src/commands.rs` and `src/commands/whitelist.rs` call them too, so there is
  one implementation. None of them reads `/dev/tty` or prints.
- `src/tui/` uses `ratatui` and `crossterm`. A pure state machine (`App` + `Msg` → `App`) is
  separate from rendering and I/O. `src/tui/term.rs` holds the terminal guard and
  `FilteredBackend`, the one write point; `src/tui/text.rs` the `Outside` values and their
  budgets; `src/tui/style.rs` the style constants (§Display integrity).
- One std thread reads terminal events and posts them as `Msg`s on a tokio channel; the `App`
  runs on the CLI's current-thread runtime (cli.rs:528). RPC, explorer and daemon calls run as
  tokio tasks and post `Msg`s back, so every pane has loading and error states.
- Warm-up uses `wallet::prepare_unlock` then `wallet::unlock` (`protocol::start_daemon` inside),
  then `wallet::open_with_password` and `wallet::check_list` (one blocking task). Sends use
  `wallet::review_transfer` then `wallet::send_transfer`. The status display uses
  `rpc::tx_state`, the detail `rpc::tx_detail`. The explorer client is `chains::evm::explorer`,
  the merge `history::merge`.
- Added 2026-10-28: `src/tui/actions.rs` holds the action table: per action its key, its words,
  its CLI alias, its tab and when it can run. The menu, the hint lines and T12 read that one
  table. The entry points of the new functions are named per unit in
  [`v0-2-tui/units.md`](v0-2-tui/units.md) (`wallet::balances` for every network, `wallet::create`,
  `wallet::repair`, `wallet::call`, `wallet::review_invoke`, `wallet::sign_message`,
  `wallet::token_balance`, a replacement in `wallet::review_transfer`, and the phrase entry point
  if trade-off B picks B2); each is the CLI handler's code moved, and the CLI calls it.

## Acceptance

Each item is checked from real frames: tmux at 120×40, captured with `capture-pane`, and the
dispatcher reads the frames. Behaviour is checked against fixed fixtures on the mock RPC node from
`tests/evm_offline.rs`, with expected values written into the fixture, not computed by the CLI.
Every `TestBackend` frame of every test is drawn through `FilteredBackend` with zero replacements
and passes the style assertion (§Display integrity rules 1 and 2). T3 and T3b are checked on
`TestBackend` frames of the `App` built in the test with a fixture explorer server, because the
binary's explorer URLs are fixed (§History); a live test (`cargo test -- --ignored`) reads
`eth.blockscout.com` through the same parser.

T12–T41 were added on 2026-10-28 for the functions of §Coverage matrix; the unit that closes
each is in [`v0-2-tui/units.md`](v0-2-tui/units.md).

| ID | Run | Observe |
|---|---|---|
| T1 | 2 keys, `b` globally active | both listed, `b` marked; lock state only in the status bar |
| T2 | Enter on `a` (locked), type the password | bar shows `signing (agent too) until` ≈ now+1h and `verified by password` for `a`; `zwallet status` agrees; the global active key is still `b` |
| T2b | `s` on `a`, confirm | `zwallet list` shows `a` active |
| T3 | fixture: 14 txs, the same hash in the journal and in Blockscout, 2 pending | exactly 10 rows; pending rows first; that hash appears once with both sources; coverage note shown |
| T3b | explorer unavailable (fixture 403) | journal rows only; the note says so; no error banner covering the list |
| T4 | Enter on a row | fields equal the fixture's expected values, token movements from the receipt included |
| T5 | USDC fixture balance | native + USDC rows; zero-balance tokens hidden |
| T6 | asset → `alice` → amount → review → type `alice` | hash shown at once; status reaches mined on the mock; journal line written |
| T6b | the payee picker | lists only the checked list's address entries valid on this chain; a non-listed address cannot be entered |
| T6c | nonce mined by another hash (fixture) | status `dropped/replaced` |
| T7 | add `bob` while warm | the change and its outcomes are shown above the password field; `whitelist list --verify` shows `bob` at gen+1; the wallet is locked afterwards, or warm for the duration chosen with the change |
| T8 | `c` → base | every pane reloads for base; the RPC host shown redacted |
| T9 | 80×24 resize; 79×24; `q`; forced panic | layout fits at 80×24; one `terminal too small` line at 79×24; the terminal is restored after both |
| T10 | `zwallet lock` from another shell | the TUI shows `locked` within two refreshes (10 s) |
| T11 | pty test across T2 and T7 | no echo; the password appears in no captured frame; cancel mid-entry, then a retry, works |
| T12 | `?`, type `whitelist add`, Enter | one menu row (key `a`, tab 5); Enter opens the add form. A test lists the CLI column of §Coverage matrix rows 1–52 and asserts each is a menu alias and each menu row is one of them; an action that cannot run shows its reason, and Enter on it changes nothing |
| T13 | from the start: `1` … `7`, `[`, `]`, Esc in a popup and in a sub-screen; every screen at 80x24 | each tab reached; every frame's hint line ends in `? actions`; Esc closes the popup, then returns to the tab; every screen of [`03-tui-screens.md`](03-tui-screens.md) fits 80x24 (`TestBackend`) |
| T14 | 7 session, `u`, `10m`, the password | the bar says `signing (agent too) until` ≈ now+10m; `zwallet status --json` has the same `expiresAt` |
| T15 | 2 wallet, `a`, mock nodes for two chains and one that fails | one line per network, testnet last; the failed one `error: … [CODE]` cut to 72 (`brief`); `2 of 3 networks have a balance; 1 did not answer`; zero tokens hidden |
| T16 | 7 session, a daemon of this home and one of another home | lock state, expiry, daemon pid, `other homes: 1 daemon`, the session's whitelist as `reports gen N, M entries (not checked here)`; with TUI-1c's argv-listed fake: its values escaped within their budgets, nothing drawn verified |
| T17 | 4 receive, warm; then locked | warm: the full EIP-55 address and `verified by password`; locked: `not verified` and the `V` hint; every chain named under it |
| T18 | `Q` on 4 receive | Q1 (default): the one line naming `zwallet address`. Q3: at 120x40 the overlay; the frame's `QR_DARK` cells decode (as src/qr.rs's tests decode, `rqrr`) to the address; an unverified address draws no code and says why; at 80x24 the size line; no cell outside the overlay has a QR style |
| T19 | 1 keys `V`; then wallet.json's cached address of `beta` edited, `V` again, then `A` and the password | every row `verified`; after the edit `V` shows the CLI's `VALIDATION_ERROR` naming `A`; `A` shows `repaired` (`was` escaped, at most 64) and every row `verified`; `zwallet list --verify` agrees |
| T20 | Linux, `kernel.yama.ptrace_scope` 0: the warm-up, then `zwallet tui --allow-ptrace-scope-0` and the warm-up | the first refuses before the password field and names the flag; the second opens the session (`prepare_unlock` called with the flag) |
| T21 | review with nonce 12 pending; then a repeat of a send from 5 min ago | the pending line and `p`; confirm without `p` signs nothing (zero `SignTx`), with `p` once; the same for the repeat with `d` |
| T22 | review, `g`, `5` | the review runs again with a 5 gwei ceiling; a fee over the automatic ceiling shows the CLI's `VALIDATION_ERROR` until `g` raises it |
| T23 | 5 whitelist: `d` on `bob`; `f` base `0.001`; `R` | each shows its summary above the password (the reset's cost spelled out); after each `whitelist list --verify` agrees; locked afterwards |
| T24 | `a`, kind contract, `router`, ethereum, `0x414bf389` | the outcome has `warning: a listed contract may forward value or act on its arguments`; `whitelist list --verify` shows the contract entry with its selector |
| T25 | `m` (off → on) | the summary shows the old and the new value (off, on); `whitelist list --verify` has `allowMessageSigning: true` |
| T26 | 2 wallet `/`, a hash on the mock; a hash the node does not know | the transaction screen (T4's fields); `not found` with the CLI's hint; a 63-digit hash cannot be entered (the field's alphabet and length) |
| T27 | send status of a pending send at nonce N, `s`, confirm | one `SignTx`: nonce N, the same `to`, `value` and `input`, fees at least 1.25x the replaced transaction's (`--bump`, README.md:615-617); the journal line; the status follows the new hash |
| T28 | the same, `x`, type `cancel` | one `SignTx`: nonce N, `to` the key's own pinned address, `value` 0, no calldata |
| T29 | the mock times out `eth_sendRawTransaction` | `BROADCAST_UNCERTAIN` on the status with `R`; `R` reviews the same send at nonce N with `--replace` the first hash; no path sends it without a nonce |
| T30 | 1 keys `n`, `trading`, the password; again with `10m` | `zwallet list --json` has `trading` active and verified; with `10m`, unlocked until about now+10m |
| T31 | an empty home, `zwallet tui`; the form with two passwords that differ, then the same | the create form; the mismatch creates nothing; then wallet.json exists, `main` is key 0, the frame shows the backup reminder and no word of the phrase |
| T32 | 1 keys `B` | B1 (default): the one line naming `zwallet backup`. B2: the password, then the words; any key hides them; so do 60 s; no other frame of the run, and no capture after `q`, holds any of the words (a test seed) |
| T33 | 6 tools call `balanceOf(address)(uint256)` on the mock; then a result of 600 characters of ESC and `█` | the result; then the result view shows at most 512 escaped characters in at most 7 rows, zero replaced cells |
| T34 | 6 tools invoke `router`, `0x414bf389`, its signature and arguments, type `router` | the review shows zwallet's decode (contract-call, contract, selector, value); one `SignTx`; the journal line has kind `invoke`. A signature whose selector `router` does not list cannot leave the form, which names the listed ones |
| T35 | invoke with raw calldata | first 4 bytes listed: as T34; not listed: refused at the form |
| T36 | 6 tools sign with message signing on; off; a paste with a non-ASCII character; 513 characters | on: the signature, checked against the address (`verify_message`, chains/evm/mod.rs:193); off: the review shows `WHITELIST_DENIED` and points to `m` on 5 whitelist; the paste and the long message are refused at the field, which names `zwallet sign-message` |
| T37 | 6 tools token, a contract on the mock whose `symbol()` is 40 characters with ESC | its balance with the symbol escaped and cut at 32; no send is offered for it |
| T38 | 4 receive, a Tron key (05-tron.md) | the Tron address with its own trust words, as 05-tron.md defines them |
| T39 | `c` tron, 2 wallet | TRX and TRC20 USDT balances, as 05-tron.md defines them |
| T40 | 3 send on tron to a listed Tron payee | as T6, with 05-tron.md's inputs, outputs and errors |
| T41 | `/` a Tron hash | as T26, with 05-tron.md's lookup |

**Tests:**
- `TestBackend` buffer assertions for each screen, through `FilteredBackend`, with the style and
  per-source budget assertions.
- `Msg`-sequence unit tests for the state machine.
- A pty end-to-end run of T2 → T6 → T7 against the mock node.
- At least one counter-example per unit ([`v0-2-tui/units.md`](v0-2-tui/units.md)): an outside
  string with a control character or a QR glyph, a 600-character name, a forged explorer row, a
  rewritten whitelist.json, a forged session view, a password in a frame.

## Changes on 2026-10-28, v2 (every function)

Against `origin/main` 24f4b5b (TUI-1a and TUI-1b merged), after the owner's words of 2026-10-28
(§Problem). Decisions of this revision, each reversible:

1. **§Coverage matrix** added: 59 rows, every CLI command and user-meaningful flag (README.md
   §Commands, src/cli.rs:36-443) and Tron. 52 reachable from the TUI (4 of them Tron, once
   05-tron.md and its library land), 3 CLI-only for a security reason (a typed payee, `--force`,
   a message the review cannot show whole), 2 the daemon never signs, 2 not applicable.
2. **§Non-goals** re-decided: the free-form payee, swaps and WalletConnect, the settings file and
   "no second implementation" stay, each with its reason; the invoke screen and message signing
   come in; the recovery phrase and the QR code are trade-offs B and Q (§Open trade-offs), with a
   recommendation each (B2, Q3) and the CLI pointer (B1, Q1) shipped until the owner picks.
3. **§Screens and keys**: seven tabs on the tab line (it replaces the title line); one action
   menu on `?` and `:` that is also the help (every action, its key, its CLI alias, its tab, why
   it cannot run now); a hint line per screen ending in `? actions`; `V` is global; text fields
   take every key but Enter, Esc, Tab, the arrows and Ctrl-C; confirms by what an action does.
   Research: [`research/tui-ux.md`](research/tui-ux.md).
4. **Mockups** in a companion file, [`03-tui-screens.md`](03-tui-screens.md), because this file
   was 692 lines before them: one per screen at 80x24, the home screen at 120x40 too.
5. **Rule 3** gains the new inputs and sources (`call`'s result, a token's own symbol, the home
   path, pids, the `repaired` list); **rule 4** a result view for `call`; **rule 5** names
   trade-off Q; **rule 7** says when a key row reads `verified`.
6. **§Status of a send** gains replacements (speed up, cancel, retry at nonce N) and the invoke
   and sign-message reviews, under the same exact-intent confirm.
7. **`zwallet tui --allow-ptrace-scope-0`**: a start flag, as the CLI's flag on `unlock`, passed to
   every session the TUI starts (T20).
8. **Acceptance** T12–T41 added. Units renumbered: TUI-3 … TUI-12
   ([`v0-2-tui/units.md`](v0-2-tui/units.md) has the old-to-new map); the explorer re-measure now
   gates TUI-7 (§History).

## Changes on 2026-10-28

Decisions of the dispatcher when TUI-1 was split into three PRs
([`v0-2-tui/units.md`](v0-2-tui/units.md) §TUI-1 split), applied in TUI-1b:

1. Status bar: two lines (§Screens and keys); the expiry in UTC, `HH:MMZ`, with the date in front
   a day or more away (the date is TUI-1b's addition: a year-long session must not read as
   ending today).
2. Rule 3: the RPC host is the origin's host (and port), at most 32 characters, cut with `...`.
3. Rule 4: a pane whose whole content is one error is an error view and wraps it.
4. §Address trust: locked, line 1 says `not verified`; `locked` is said once, on line 2 (T1).
5. Status bar, after TUI-1b's review round 1: line 2 says `signing (agent too) until`, not
   `signing open (agent too) until`, and drops `rpc `, so the date form (17), the words (26), a
   separator and a 32-character host fit 78 columns; every value is whole or cut with `...`;
   `q` waits for a running `L`; a status read from before the lock is dropped.
6. After TUI-1b's review round 2: key reads carry the generation too; after a lock the bar
   says `checking...` until a fresh status lands; the key list narrows the name for an index of
   four digits or more.
7. After TUI-1b's review round 3: line 1's key name is sized from the trust words drawn in each
   state (at least 8), so an impostor's warning words are never lost to a long name.

## Changes on 2026-10-27

Each fix below is something today's code (c7d3f13) contradicts, something that did not exist when
the 2026-10-20 text was written, or a decision of review round 1, 2 or 3 (marked R1, R2, R3).

1. Status: the whitelist shipped (PR #4). TUI-1 starts once the agent-side v0.1 work is merged;
   G3 and G4 (v0-1/loop.md:27-28) wait on owner actions only (R1).
2. T2: `zwallet tui` and `--warm` are new; neither exists (cli.rs:36-88).
3. T2 / Architecture: the warm-up runs `unlock`'s checks and pins the binary before the password
   field opens, not `protocol::start_daemon` alone (commands.rs:1913-1922).
4. Status bar: address trust added. Readiness carries only the globally active key's address
   (daemon/mod.rs:367-369), so the TUI derives the open key's address from the seed itself with
   the typed password (§Password check, R1); Linux never verifies by session
   (core/protocol.rs:1170). The password-checked addresses are pinned (R1).
5. T4: the fee needs `effectiveGasPrice` (and `l1Fee` on base and optimism), the confirmations a
   head block; `lookup_tx` reads neither (chains/evm/rpc.rs:1342-1346).
6. T4: the journal keeps no daemon-decoded view (chains/mod.rs:1175-1192); "local record" shows
   the journal's own fields.
7. T6: sends go through new entry points; today's `sign_and_send` is private, takes `&SendArgs`,
   and prints to stderr during the broadcast (commands.rs:1631, chains/mod.rs:1215).
8. T6: the review takes no lock and no signature; confirm re-prepares under the lock, runs the
   pending and duplicate guards again, and signs only when the transfer intent is exactly equal
   to the review's and the most the fee can cost is at most the reviewed one (R1, R2, §Status of
   a send).
9. T6, T7, T4: payee entries, T7's verified state and T4's nickname labels come only from the
   list the TUI checked with the password; the session's view (no peer check,
   core/protocol.rs:434-436) and the file alone are never trusted for them; a changed file drops
   the checked list (R1, §Password check). The 2026-10-27 first draft gated the picker on the
   session's view, which does not authenticate an address.
10. `dropped/replaced` has no code path today (chains/evm/rpc.rs:1315-1319; `mined_at`, :965, is
    private); flow.md lists it as missing.
11. T7: without the password `whitelist list` is `verified: false` (commands/whitelist.rs:1416);
    `V` checks the list with the password and changes nothing else (R1).
12. T7: the whitelist key comes from the password and a sealed salt; there is no key file
    (core/whitelist.rs:727, 04-whitelist.md:104). Reset added to the screen.
13. T8: no "last chain used": it needs a settings file, a non-goal; the code has only
    `PRIMARY_CHAIN` (chains/mod.rs:145).
14. History: re-measured. Sepolia now answers; optimism moved to `explorer.optimism.io`; bsc and
    avalanche added (no Blockscout); Routescan probed; a re-measure gates TUI-3 (R1).
15. History: the explorer's shape check, kept and dropped fields, and the 1 MiB / 8 s bounds;
    a 0-value incoming transaction is dropped like a 0-value token transfer (R1).
16. Display integrity section added: ASCII chrome, the password mask `*` (was `●`), no QR code,
    styles from code, one line per outside value, nothing printed while the TUI runs.
17. Size: below 80×24 one `terminal too small` line (T9 extended).
18. Acceptance: T3 and T3b run on in-test frames (the explorer URL cannot be overridden); every
    frame goes through the write point with zero replacements; T10 within two refreshes (R1).
19. Password: keystrokes go into the buffer from the input thread; a `Msg` carries only the
    length; a paste's copy is zeroized; no `Debug`, `Display` or `Clone`; password mode starts
    with the Enter that opens the field; the panic hook prints a fixed line (R1).
20. Display integrity rule 1 is enforced at run time by `FilteredBackend`, not only in tests (R1).
21. Rule 5 rewritten: no claim that excluding block glyphs makes a code impossible; the defence
    is styles, placement and budgets together, each tested (R1).
22. Rule 3: per-source budget assertions apart from the cell rule; the environment's RPC URL,
    the session view's entries and the input fields' alphabets added (R1).
23. Rule 4: the error view wraps the already escaped text, so an outside newline adds no row (R1).
24. Rule 7: the one rule for drawing "verified" (R1).
25. T4: token movements decoded from the selected transaction's receipt, so a transfer routed
    through a contract shows its real recipient and amount from the node (R1).
26. Status of a send: the 90 s bound becomes `unknown` in the TUI's own state, since
    `record_receipt` reports `pending` (chains/mod.rs:1396-1399) (R1).
27. Units renamed TUI-1 … TUI-6 (R1).
28. Rule 1: the filter is tested where it acts (the cells the inner backend receives); a glyph
    two columns wide becomes `?` and a space; terminal control from data is ruled out, the
    backend's own cursor and style sequences are not (R2).
29. Status of a send: confirm signs when the re-prepared most fee is equal to or lower than the
    reviewed one; every other held field must be exactly equal (R2).
30. Password check: the fingerprint is checked when a pane that shows the checked list opens or
    refreshes, when the picker opens, before the review, and at confirm (R2).
31. Password check: two entry points, `wallet::open_with_password` and `wallet::check_list`;
    `address --verify` keeps its repairing path and reads no list file (R2).
32. T4: a token movement of amount 0 is counted, not shown (R2).
33. Rule 4: the escaping reference is `escape` (core/text.rs:40-50), run by `ascii_inert` (:69)
    (R2).
34. Password check: the scrypt run happens in `spawn_blocking` with a `checking...` state (R2).
35. Status of a send: the whitelist verdict and the fee cap are computed on the checked list,
    not on whitelist.json at that moment (R2).
36. Password check: a cache modified before the warm-up fails the warm-up (daemon/mod.rs:311);
    one modified between the session start and the check leaves the session and pins nothing
    (R2).
37. Status of a send: the fee compared at confirm is `max_gas_cost` plus `l1_fee` as U256
    integers; the sequencer sets `l1_fee` at inclusion; a rise that leaves the balance short is
    caught by `sufficientFunds` (R3).
38. Password check: `open_with_password` returns the anchor and a `salted` flag, never the salt;
    the checked list's policy is the list's own (R3).
