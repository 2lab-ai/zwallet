# zwallet — TUI

Status: **planned** (v0.2), revised 2026-10-20 after two spec reviews, revised 2026-10-27 against
PRs #6–#12 (`origin/main` c7d3f13) and after review rounds 1 and 2 the same day, and on
2026-10-28 for TUI-1b (§Changes on 2026-10-28). The signed
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
Transfer and Whitelist, while the list is not checked: V = check it with the password (nothing else)
```

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
  - A guard failure shows its message. An override (allow-pending or allow-duplicate) is a
    separate key press, with the warning on screen.
- **Terminal:** `zwallet tui` needs stdin on a terminal (raw mode is set on stdin, and the panic
  hook restores stdin's modes) and stdout on one; otherwise it refuses before taking either.
- **Size:** below 80 columns or 24 rows the TUI draws one line, `terminal too small (80x24
  needed)`, and nothing else. At 80x24: a title line, the pane (bordered with `+ - |`, 78 columns
  inside), a key-hint line, and the two bar lines.

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
whether ratatui's diff sends that column. Tests draw through the same wrapper over ratatui's
`TestBackend` and assert zero replacements on every frame: the wrapper is the last filter, as
`write_human` is, and escaping where text comes in (rule 3) stays mandatory. The TUI's own chrome
keeps to the set too: borders are `+`, `-` and `|`, the selection marker is `>`, the password mask
is `*`, and busy states end in `...` (`checking...`, never `…`).

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
| what the human types | input fields | each field takes only its own alphabet at the key event: an amount `0-9` and one `.`; a nickname `a-z 0-9 _ -`, at most 32 (`validate_nickname`, core/whitelist.rs:455); an address `0x` then `0-9 a-f A-F`, at most 42; a duration `0-9` and one of `s m h d w y`; the password any byte, shown only as `*`. Any other key changes nothing and nothing else is echoed |

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
an error line), or a pane whose whole content is one error (the keys pane when the key list
cannot be read):
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
rule 2; the budget assertions of rule 3.

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
about the whitelist, and nothing a file says, is ever drawn as verified.

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

Before TUI-3 merges the table is measured again, Routescan's other paths for base, arbitrum and
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
  the recorded modes are restored and the process ends. A panic on another thread then (which
  that order rules out) gets the same, without waiting for the taking thread. `_exit` runs no destructors, so a `Zeroizing` buffer such
  as `PasswordBuf` is not cleared on that path: its memory goes with the process.

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

## Acceptance

Each item is checked from real frames: tmux at 120×40, captured with `capture-pane`, and the
dispatcher reads the frames. Behaviour is checked against fixed fixtures on the mock RPC node from
`tests/evm_offline.rs`, with expected values written into the fixture, not computed by the CLI.
Every `TestBackend` frame of every test is drawn through `FilteredBackend` with zero replacements
and passes the style assertion (§Display integrity rules 1 and 2). T3 and T3b are checked on
`TestBackend` frames of the `App` built in the test with a fixture explorer server, because the
binary's explorer URLs are fixed (§History); a live test (`cargo test -- --ignored`) reads
`eth.blockscout.com` through the same parser.

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

**Tests:**
- `TestBackend` buffer assertions for each screen, through `FilteredBackend`, with the style and
  per-source budget assertions.
- `Msg`-sequence unit tests for the state machine.
- A pty end-to-end run of T2 → T6 → T7 against the mock node.
- At least one counter-example per unit ([`v0-2-tui/units.md`](v0-2-tui/units.md)): an outside
  string with a control character or a QR glyph, a 600-character name, a forged explorer row, a
  rewritten whitelist.json, a forged session view, a password in a frame.

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
