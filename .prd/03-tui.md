# zwallet — TUI

Status: **planned** (v0.2), revised 2026-10-20 after two spec reviews, revised 2026-10-27 against
PRs #6–#12 (`origin/main` c7d3f13). The signed whitelist shipped in v0.1 (WU1b, PR #4,
[`04-whitelist.md`](04-whitelist.md)). Work starts after v0.1 is complete; its loop still has two
open gaps, G3 (the stable xbrew release) and G4 (A12, the user's unlock)
([`v0-1/loop.md`](v0-1/loop.md)). Fixed user statement: [`v0-2-tui/ssot.md`](v0-2-tui/ssot.md).
What the TUI would call today, function by function: [`v0-2-tui/flow.md`](v0-2-tui/flow.md).
Units and order: [`v0-2-tui/units.md`](v0-2-tui/units.md).

## Problem

Every zwallet command is a one-shot CLI call with JSON output, which suits an agent. A human who
wants to check the wallet, approve a send or curate payees has to remember flags and read JSON.
On 2026-10-20 the user asked for `zwallet tui` to do the following: list the wallets, warm one up
with the password for about an hour, show the balance and the last 10 transactions, show a
transaction in detail, transfer an asset to a whitelisted payee, and check the result by tx hash.

## Goals

- **T1 Keys.** Opens on the key list (the master's named subkeys), with the globally active key
  marked and each row showing its address. Lock state is shown once, in the status bar, because it
  belongs to the master and not to a key. The row addresses are never marked verified (`list`
  without the password is never verified, flow.md §T1).
- **T2 Warm-up.** Enter opens a key *in the TUI* (an internal `--key`, not a global switch). If the
  wallet is locked, it asks for the password and unlocks for **1h** through the staged unlock.
  `zwallet tui --warm <dur>` changes the 1h; `zwallet tui` and `--warm` are new (flow.md Fact 5).
  Making the key globally active is a separate key press (`s`) with its own confirmation, because
  the active key is what the agent uses by default.
- **T3 Home.** The balance summary, plus the last 10 transactions for the current chain (§History).
- **T4 Tx detail.**
  - Fields: hash, from/to (nickname if listed), nonce, value and asset, fee
    (`gasUsed × effectiveGasPrice`, plus the receipt's `l1Fee` on base and optimism), block,
    confirmations, status, explorer URL. Today's `tx` lookup gives none of the fee, the
    confirmations or a token's amount (flow.md §T4).
  - For this machine's sends it also shows the send journal's own fields (kind, amount, recipient,
    nonce, time), labelled "local record".
- **T5 Assets.** Enter on the balance lists the native coin plus the registry tokens (USDT, USDC,
  DAI, WETH) with a non-zero balance.
- **T6 Transfer.** Enter on an asset starts a transfer:
  1. pick the recipient from the whitelist's **address entries valid on this chain** (§Payee
     picker);
  2. enter the amount;
  3. review the send;
  4. confirm by typing the payee's nickname;
  5. the tx hash appears at once, and the screen follows its status (§Status of a send).
- **T7 Whitelist.** List the entries (gen, `verified` state, policy), add an address entry or a
  contract entry with selectors, remove an entry, set the fee cap, reset.
  - Every change asks for the password, even when warm, and runs the 04 edit protocol
    (§Whitelist edits).
  - The add screen shows the full EIP-55 address and never pre-fills an address from history
    (address-poisoning defence).
- **T8 Chain.** A chain switcher. The default is `--chain <key>` when given, otherwise ethereum.

## Non-goals

- No mnemonic display and no backup screen.
- No free-form recipient field: no payee outside the whitelist, ever.
- No swaps, no `invoke` screen, no WalletConnect.
- No settings file.
- No QR code. `zwallet address` prints one for a verified address (README.md:228-240).
- No second implementation: the TUI calls the same library code as the CLI. The send lock, the
  journal, the guards and the daemon whitelist all apply unchanged.

## Screens and keys

```
Keys ──Enter──▶ (password if locked) ──▶ Home [balance | recent tx] ──Enter on tx──▶ Tx detail
  │ s = make active (confirm)              ├─Enter on balance──▶ Assets ──Enter──▶ Transfer:
  │                                        │     payee (address entries) → amount → review → type nickname → status
  │                                        ├─ w ──▶ Whitelist (a add · d remove · f fee cap · R reset; password per change)
  │                                        └─ c ──▶ Chain picker
Global: ↑/↓ j/k · Enter · Esc back · r refresh · L lock (all daemons) · ? help · q quit
```

- **Status bar:** chain · key · short address · address trust (§Address trust) ·
  **`signing open (agent too) until 14:32`** or `locked` · RPC host (redacted). The warm-up opens
  signing for every client of the daemon, including the agent, and the bar says so. The lock
  state is read again every 5 s and on `r`.
- **Review screen:**
  - payee nickname and full address, asset and amount;
  - the fee estimate and the fee cap;
  - the whitelist verdict, the pending guard and the duplicate guard (02 §Send safety).
  - A guard failure shows its message. An override (allow-pending or allow-duplicate) is a
    separate key press, with the warning on screen.
- **Size:** below 80 columns or 24 rows the TUI draws one line, `terminal too small (80x24
  needed)`, and nothing else.

## Display integrity in the TUI

The CLI writes every human byte through one filter, `core::terminal::write_human`
(core/terminal.rs:44). The TUI does not: ratatui puts characters into cells and writes the cells
to the terminal itself. So the TUI states the same rules for its cells. The threat is the one the
README states: a process of this user controls some of the text zwallet shows, and must never make
zwallet's own screen show its address as verified or fundable: no QR code, no forged "verified"
mark, no terminal control.

**1. Every cell is in the write point's set.** A cell holds one character of printable ASCII
(`0x20`–`0x7E`) or `₮`: `TERMINAL_CHARS` (core/terminal.rs:20) without the newline, which a cell
never holds. The test is `terminal_ok` (:24) on every cell of every frame. The TUI's own chrome
keeps to it too: borders are drawn with `+`, `-` and `|`, the selection marker is `>`, and the
password mask is `*`. So a frame can never carry a block glyph, a box-drawing glyph, ESC, a bidi
control or any other character outside that set, whatever the data.

**2. Colour and style come from code only.** Each widget's style is a constant in `src/tui/`.
No data chooses a colour, a reverse video or a background: a QR code can be drawn with background
colours on spaces.

**3. Every string from outside is made inert and bounded where it comes in, with the CLI's own
functions and budgets.** "From outside" means text zwallet does not write: a file in the home that
any process of this user can edit, a reply on `d.sock`, an RPC endpoint, the explorer.

| Source | What the TUI shows from it | Check and budget (the CLI's own) |
|---|---|---|
| `wallet.json` key names and cached addresses | T1 rows, the status bar | names (and `active`) pass `validate_key_name` when wallet.json is parsed (core/store.rs:568-576), else the file is refused; a cached value that is not an address is an error that shows it escaped, at most 64 (`cached_address`, commands.rs:251; `VALUE_SHOWN`, :195) |
| `d.sock`: an address | T1 rows, the status bar | `reply_address` (commands.rs:267): an address's shape, else an error that shows it escaped, at most 64 |
| `d.sock`: the session's expiry | the status bar | `shown`, at most 64 (commands.rs:1827) |
| `d.sock`: the session's whitelist view | T7, the payee picker | `inert_whitelist` (commands.rs:208): every string escaped, `reason` at most 512 |
| `d.sock`: an error reply | any error line | `relayed_error` (core/protocol.rs:329): the message and its kept details share 512; from another process, named, details dropped |
| `d.sock`: who answered | the status bar's warning | `PeerCheck::warning` (core/protocol.rs:1042); the executable path at most 200 (`EXE_SHOWN`, :1052) |
| `whitelist.json`, `whitelist.next.json` | T6 picker, T7 | shape-checked at parse (`check_shapes`, core/whitelist.rs:689: nickname `a-z 0-9 _ -`, `0x` hex address and selectors, chain-key shape, decimal caps), else not a list; the parser's message quotes at most 80 (`clip`, :450); names and fee caps from the files share one 512 budget (`FileTextBudget`, commands/whitelist.rs:899); one field at most 64 (`SHOWN_CHARS`, :1236); a verified list up to 100 entries, an unverified one within one 512 budget (`list_text`, :1459-1520); chains or selectors at most 16 per entry (`ITEMS_SHOWN`, :1244) |
| the send journal | T3 rows, T4 local record, T6 guards | each field in the shape zwallet writes it, else `<invalid>` (`EntryText::of`, chains/mod.rs:756); at most 512 per message (`JOURNAL_TEXT_MAX`, :692) |
| an RPC endpoint's error | T3 network line, T4, T6 | scrubbed of the URL's key (`Net::redact`, chains/mod.rs:282), escaped and cut to 512 where the error is built (`failed`, chains/evm/rpc.rs:109-140); a network's line cut to 72 (`brief`, commands.rs:1143, :1355) |
| the explorer | T3 rows | the shape check in §History; no free text from it is ever shown |

`core::text::escape` turns `₮` into `\u{20ae}` like every non-ASCII character (core/text.rs:40-50;
its test, :229), so `₮` reaches a cell only from zwallet's own registry constants
(chains/evm/registry.rs:71, chains/evm/rpc.rs:237-240) or from a journal amount in the shape
zwallet writes (chains/mod.rs:739).

**4. One source never fills enough rows to draw a code.** A QR code needs at least 21 × 21
modules (version 1; an Ethereum address needs version 2, 25 × 25). In list panes and the status
bar each outside value sits on one line, cut to the pane's width, never wrapped. Only the error
detail view (Enter on an error line) wraps text, one source alone at a time, at the pane's inner
width of at least 78 (the size rule keeps the terminal at 80 columns or more): 512 characters fill
at most ⌈512 / 78⌉ = 7 rows. Two outside sources are never wrapped in one block. So the residual
the README states for a narrow terminal (README.md:150-205: a 512-character source on a
21-column terminal) does not reach the TUI.

**5. The TUI draws no QR code**, and no "verified" mark for an address unless the session or the
password verified it (§Address trust). With rule 1 this is structural: no cell can hold a block
glyph, so no frame holds a code, zwallet's or anyone's.

**6. Nothing prints while the TUI holds the terminal.** Four places in today's code write to the
terminal from inside the logic (flow.md Fact 3): the `broadcasting <hash>` line of every live send
(chains/mod.rs:1215), `create`'s backup note (commands.rs:752-755), a whitelist summary with
`--password-stdin` (commands/whitelist.rs:264) and the password prompt (core/password.rs:152).
The entry points the TUI calls return that text instead; the TUI shows it in a pane, through
rules 1–4.

**Addresses a human may copy or pay** (the payee picker, the review, the add screen, T4's from
and to) are always shown in full EIP-55, rechecksummed by zwallet. Only the status bar shortens
the wallet's own address.

## Address trust and session verification

The TUI shows an address's trust exactly as the CLI's `verified` and `verifiedBy` say it
(README.md:92-149; `Trust`, commands.rs:371-411), on the status bar, for the key open in the
TUI:

| Bar | When | Code |
|---|---|---|
| `verified by password` | the warm-up this TUI just ran reported this key's address. Readiness carries the **globally active** key's address (daemon/mod.rs:367-369), so this holds only when the open key is the active one | `unlock`'s `verifiedBy: "password"` (commands.rs:1934-1936) |
| `verified by session` | unlocked, and every answer for the address came from this build's key daemon of this home (macOS only) | `Trust::Session` (commands.rs:555-566), `check_peer` (core/protocol.rs:1078) |
| `not verified (locked)` | locked: the cache | `Trust::Locked` |
| `not verified` | unlocked, but no session check passed: a cached address, or Linux, where the daemon's executable is hidden and the check never passes (core/protocol.rs:1170, :745-751). On Linux the bar adds `(no session check on Linux)` | `Trust::Unverified(None)` |
| `NOT this build's key daemon: pid N ...; run zwallet lock and unlock again` | another process answered on `d.sock` | `Trust::Unverified(Some(warning))`, `PeerCheck::warning` (core/protocol.rs:1042) |

The last line replaces the trust state, cut to the bar's width; Enter on it shows the whole
warning (within its own budget, §Display integrity rule 4). A cache that the daemon's answer
contradicts stops the pane with the CLI's `VALIDATION_ERROR` (commands.rs:520-536): fund neither
address.

## Payee picker

The picker lists the address entries of `whitelist.json` that apply on this chain
(`Whitelist::payee`, core/whitelist.rs:255), read and shape-checked by `read_list`
(core/whitelist.rs:860). The file is unverified, and the key daemon decides every send. So the
picker offers an entry only when the session vouches for the file: unlocked, the serving daemon's
whitelist view (`wallet::status`, flow.md §T7) says `verified: true` at the same `gen` as the file,
and lists the entry's nickname. Otherwise the picker offers nothing and says why (locked; or
"whitelist.json (gen N) is not the list this session signs under (gen M)"; or the session's
reason). This keeps a planted `whitelist.json` (a lookalike address under a known nickname) from
reaching the review screen.

## Whitelist edits

Every change in the TUI runs the edit protocol of 04-whitelist.md §Edits, through the two entry
points flow.md §T7 names, and nothing else:

1. `wallet::whitelist_plan(home, change, unlock)`: checks the change as the list will, runs
   `unlock`'s own checks and pins the daemon binary when the human chose a duration, reads both
   list files once, and returns the summary of what the password authorizes with the files'
   fingerprint (step 0; commands/whitelist.rs:240-262). The summary is drawn above the password
   field. It is bounded already (`FileTextBudget`, commands/whitelist.rs:899) and passes the cell
   rules. Nothing is stopped yet.
2. The password is typed for this change, even while warm: the TUI keeps no password once a
   session has started (§Password handling).
3. `wallet::whitelist_apply(plan, password)` (steps 1–5; commands/whitelist.rs:269-318): the
   password is checked against `wallet.json` (a wrong one stops nothing); the whitelist key is
   derived from the password and the salt sealed in `wallet.json` (core/whitelist.rs:727,
   core/store.rs:391; there is no `whitelist.key`, 04-whitelist.md:104); the files must still be
   the bytes shown; **every zwallet daemon of this user is stopped** (all homes, the `zwallet
   lock` path, core/protocol.rs:1783); the edit runs under `w.lock`; and the wallet is **left
   locked**, unless the human typed a duration with the change (`--unlock <dur>`), when a new
   session runs for that duration from now. No duration or expiry is taken from the session that
   was running.
4. The TUI shows the outcome (generation, entries, stopped daemons, warning, session) and every
   pane reloads: the old session has ended.

## History (re-measured 2026-10-27)

Sources:
1. **The send journal.** This is a *local claim*. Every row from it is looked up on chain (`tx`)
   before it shows a status.
2. **A keyless explorer**, where one answers a non-browser client: `eth.blockscout.com`,
   `explorer.optimism.io` and `eth-sepolia.blockscout.com`. Base, arbitrum and polygon Blockscout
   return a Cloudflare challenge (HTTP 403); bsc and avalanche have no Blockscout instance.

**Reachability, measured 2026-10-27 11:53:24Z** with `curl 8.7.1`, no key, `GET
/api/v2/addresses/0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045/transactions`; rechecked at
11:54:29Z with no `User-Agent` header, as reqwest 0.13.5 sends by default (its client builder
sets only `Accept`: the crate's own source, `reqwest-0.13.5/src/async_impl/client.rs:284-285` in the
cargo registry, not a zwallet file):

| Chain | Host | HTTP | Content-Type | Notes |
|---|---|---|---|---|
| ethereum | `eth.blockscout.com` | 200 | `application/json` | 50 items, 523 kB; `token-transfers` 200; no-UA 200 |
| base | `base.blockscout.com` | 403 | `text/html` | Cloudflare "Just a moment..."; `token-transfers` 403; no-UA 403 |
| arbitrum | `arbitrum.blockscout.com` | 403 | `text/html` | Cloudflare challenge; `token-transfers` 403 |
| optimism | `optimism.blockscout.com` | 301 | `text/html` | redirects to `explorer.optimism.io` |
| optimism | `explorer.optimism.io` | 200 | `application/json` | 50 items; no-UA 200 |
| polygon | `polygon.blockscout.com` | 403 | `text/html` | Cloudflare challenge; `token-transfers` 403 |
| bsc | `bsc.blockscout.com` | 404 | `text/plain` | "default backend - 404": no instance. Routescan answers `chain not supported` |
| avalanche | `avalanche.blockscout.com` | 404 | `text/plain` | no instance. Routescan `api.routescan.io/v2/network/mainnet/evm/43114/address/<a>/transactions` 200 JSON (no-UA 200): another format, not used in v0.2 |
| sepolia | `eth-sepolia.blockscout.com` | 200 | `application/json` | 50 items; `token-transfers` 200; no-UA 200 |

The coverage note per chain follows this table: ethereum, optimism and sepolia show the explorer
and the journal; base, arbitrum, polygon, bsc and avalanche show "this machine's sends only;
incoming transfers not indexed".

ERC-20 log scanning was dropped. Public nodes only serve about 5k blocks of logs (roughly 20 min on
arbitrum, about 3 h on base). Those logs are also where address-poisoning `Transfer(you,
lookalike, 0)` rows show up.

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
  - A token transfer of a token outside the registry, of an NFT, or of 0 is dropped.
- **Never read into anything shown:** every name or label the explorer puts on an address
  (`name`, `ens_domain_name`, `public_tags`, `private_tags`, `watchlist_names`, `metadata`,
  `is_scam`, `reputation`, `implementations`), `method`, `decoded_input`, `raw_input`,
  `revert_reason`, `transaction_tag`, `token.name`, `token.symbol`, `token.decimals`,
  `token.icon_url`, `exchange_rate`, `historic_exchange_rate`, `confirmations` (the TUI reads its
  own from the node), and every other field. All of these were in the 2026-10-27 bodies.
- **Counted, not shown.** The coverage note says how many explorer items were dropped ("3 explorer
  rows not shown: not in the shape zwallet reads").
- **Rows are claims.** An explorer row is labelled `explorer`. T4 reads the transaction from the
  node; when the node does not know it, the detail says "the explorer lists it; this node does
  not know it". No explorer value ever fills an address field (T7 add never pre-fills from
  history).

**Merge contract:**
- Rows are keyed by `(chainId, txHash)` and de-duplicated. One row can carry several token
  movements, and lists every source that reported it.
- Order: pending first, then block number descending, then by position in the block.
- The home view shows the first 10.
- A coverage note is always visible, for example "base: this machine's sends only; incoming
  transfers not indexed".

## Status of a send

States: `pending → mined (success | reverted)`. A tx becomes **`dropped/replaced`** when its nonce
has been mined by a different hash: the account's mined nonce (`eth_getTransactionCount` at
`latest`) is past the tx's nonce and the tx has no receipt. No code tells this today (flow.md §T6:
`lookup_tx` knows only mined, pending and not found). The screen polls every 3 s (as
`RECEIPT_POLL`, chains/evm/rpc.rs:28); after 90 s (as `--wait`, commands.rs:36) it says `unknown —
run zwallet tx`. Leaving the screen never cancels anything; the journal has already recorded the
send.

**Review and confirm.** The review runs `prepare` as a dry-run, the duplicate check and the
whitelist check on the list on disk; it takes no send lock and asks the daemon to sign nothing.
Confirm takes the send lock (`SendLock`, chains/mod.rs:651), runs `prepare` again (fresh nonce and
fees), and if the nonce, the most the fee can cost (`maxGasCost`), `sufficientFunds` or the
whitelist verdict differ from what the human reviewed, it signs nothing and shows the new review.

## Password handling

- The terminal is already in raw mode, so there is no echo, before the field takes input.
- The password has one owner: a fixed-capacity `Zeroizing` buffer of 1024 bytes (the CLI's
  `MAX_SECRET`, core/password.rs:13), rendered as `*`. The input thread writes each keystroke and
  each paste into that buffer itself and posts only the new length as a `Msg`; a paste's own copy
  is dropped at once.
- No `Debug` or logging of any message that carries the password, and no event queue that keeps
  it.
- Before the field opens for a warm-up, `wallet::prepare_unlock` runs `unlock`'s checks and pins
  the daemon binary (commands.rs:1913-1922), so a binary swapped in while the human types is never
  started.
- The buffer is cleared on success, cancel and every failure path. When a whitelist edit also
  warms up (`--unlock <dur>`), it is kept only until that new session has started, then
  cleared.
- A panic hook and normal unwinding both restore the terminal.

## Architecture

- `src/wallet.rs` (new): the library entry points [`v0-2-tui/flow.md`](v0-2-tui/flow.md) names.
  The CLI handlers in `src/commands.rs` and `src/commands/whitelist.rs` call them too, so there is
  one implementation. None of them reads `/dev/tty` or prints.
- `src/tui/` uses `ratatui` and `crossterm`. A pure state machine (`App` + `Msg` → `App`) is
  separate from rendering and I/O. `src/tui/text.rs` holds the cell rules (§Display integrity).
- One std thread reads terminal events and posts them as `Msg`s on a tokio channel; the `App`
  runs on the CLI's current-thread runtime (cli.rs:528). RPC, explorer and daemon calls run as
  tokio tasks and post `Msg`s back, so every pane has loading and error states.
- Warm-up uses `wallet::prepare_unlock` then `wallet::unlock` (`protocol::start_daemon` inside).
  Sends use `wallet::review_transfer` then `wallet::send_transfer`. The status display uses
  `rpc::tx_state`. The explorer client is `chains::evm::explorer`, the merge `history::merge`.

## Acceptance

Each item is checked from real frames: tmux at 120×40, captured with `capture-pane`, and the
dispatcher reads the frames. Behaviour is checked against fixed fixtures on the mock RPC node from
`tests/evm_offline.rs`, with expected values written into the fixture, not computed by the CLI.
Every `TestBackend` frame of every test also passes the cell rule (§Display integrity rule 1).
T3 and T3b are checked on `TestBackend` frames of the `App` built in the test with a fixture
explorer server, because the binary's explorer URLs are fixed (§History); a live test (`cargo test
-- --ignored`) reads `eth.blockscout.com` through the same parser.

| ID | Run | Observe |
|---|---|---|
| T1 | 2 keys, `b` globally active | both listed, `b` marked; lock state only in the status bar |
| T2 | Enter on `a` (locked), type the password | bar shows `signing open (agent too) until` ≈ now+1h and `a`'s trust (`not verified` or `verified by session`, since `b` is active); `zwallet status` agrees; the global active key is still `b` |
| T2b | `s` on `a`, confirm | `zwallet list` shows `a` active |
| T3 | fixture: 14 txs, the same hash in the journal and in Blockscout, 2 pending | exactly 10 rows; pending rows first; that hash appears once with both sources; coverage note shown |
| T3b | explorer unavailable (fixture 403) | journal rows only; the note says so; no error banner covering the list |
| T4 | Enter on a row | fields equal the fixture's expected values |
| T5 | USDC fixture balance | native + USDC rows; zero-balance tokens hidden |
| T6 | asset → `alice` → amount → review → type `alice` | hash shown at once; status reaches mined on the mock; journal line written |
| T6b | the payee picker | lists only address entries valid on this chain; a non-listed address cannot be entered |
| T6c | nonce mined by another hash (fixture) | status `dropped/replaced` |
| T7 | add `bob` while warm | the change and its outcomes are shown above the password field; `whitelist list --verify` shows `bob` at gen+1; the wallet is locked afterwards, or warm for the duration chosen with the change |
| T8 | `c` → base | every pane reloads for base; the RPC host shown redacted |
| T9 | 80×24 resize; 79×24; `q`; forced panic | layout fits at 80×24; one `terminal too small` line at 79×24; the terminal is restored after both |
| T10 | `zwallet lock` from another shell | the TUI shows `locked` within one refresh (5 s) |
| T11 | pty test across T2 and T7 | no echo; the password appears in no captured frame; cancel mid-entry, then a retry, works |

**Tests:**
- `TestBackend` buffer assertions for each screen, each with the cell rule.
- `Msg`-sequence unit tests for the state machine.
- A pty end-to-end run of T2 → T6 → T7 against the mock node.
- At least one counter-example per unit ([`v0-2-tui/units.md`](v0-2-tui/units.md)): an outside
  string with a control character or a QR glyph, a 600-character name, a forged explorer row, a
  password in a frame.

## Changes on 2026-10-27

Each fix below is something today's code (c7d3f13) contradicts, or something that did not exist
when the 2026-10-20 text was written.

1. Status: the whitelist shipped (PR #4); v0.1 still has G3 and G4 open (v0-1/loop.md:27-28).
2. T2: `zwallet tui` and `--warm` are new; neither exists (cli.rs:36-88).
3. T2 / Architecture: the warm-up runs `unlock`'s checks and pins the binary before the password
   field opens, not `protocol::start_daemon` alone (commands.rs:1913-1922).
4. Status bar: address trust added. The warm-up verifies by password only the globally active
   key's address (daemon/mod.rs:367-369); Linux never verifies by session (core/protocol.rs:1170).
5. T4: the fee needs `effectiveGasPrice` (and `l1Fee` on base and optimism), the confirmations a
   head block, a token's amount a decode; `lookup_tx` reads none of them
   (chains/evm/rpc.rs:1309-1311, :1342-1346).
6. T4: the journal keeps no daemon-decoded view (chains/mod.rs:1175-1192); "local record" shows
   the journal's own fields.
7. T6: sends go through new entry points; today's `sign_and_send` is private, takes `&SendArgs`,
   and prints to stderr during the broadcast (commands.rs:1631, chains/mod.rs:1215).
8. T6: the review takes no lock and no signature; confirm re-prepares under the lock (§Status of
   a send).
9. T6: the payee picker offers an entry only when the session verified the file's generation
   (§Payee picker).
10. `dropped/replaced` has no code path today (chains/evm/rpc.rs:1315-1319; `mined_at`, :965, is
    private); flow.md lists it as missing.
11. T7: without the password `whitelist list` is `verified: false` (commands/whitelist.rs:1416);
    the verified state comes from the session's view or a password check.
12. T7: the whitelist key comes from the password and a sealed salt; there is no key file
    (core/whitelist.rs:727, 04-whitelist.md:104). Reset added to the screen.
13. T8: no "last chain used": it needs a settings file, a non-goal; the code has only
    `PRIMARY_CHAIN` (chains/mod.rs:145).
14. History: re-measured. Sepolia now answers; optimism moved to `explorer.optimism.io`; bsc and
    avalanche added (no Blockscout); Routescan measured for avalanche and not used.
15. History: the explorer's shape check, kept and dropped fields, and the 1 MiB / 8 s bounds.
16. Display integrity section added: ASCII chrome, the password mask `*` (was `●`), no QR code,
    styles from code, one line per outside value, nothing printed while the TUI runs.
17. Size: below 80×24 one `terminal too small` line (T9 extended).
18. Acceptance: T3 and T3b run on in-test frames (the explorer URL cannot be overridden); every
    frame passes the cell rule; T10's refresh is 5 s.
19. Password: keystrokes go into the buffer from the input thread; a `Msg` carries only the
    length.
