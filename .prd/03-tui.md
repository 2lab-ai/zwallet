# zwallet — TUI

Status: **planned** (v0.2), revised 2026-10-20 after two spec reviews. Work starts after v0.1
ships: PR #1, the signed whitelist ([`04-whitelist.md`](04-whitelist.md), v0.1 WU1b), the
distribution, and the v0.1.0 release. Fixed user statement: [`v0-2-tui/ssot.md`](v0-2-tui/ssot.md).

## Problem

Every zwallet command is a one-shot CLI call with JSON output, which suits an agent. A human who
wants to check the wallet, approve a send or curate payees has to remember flags and read JSON.
On 2026-10-20 the user asked for `zwallet tui` to do the following: list the wallets, warm one up
with the password for about an hour, show the balance and the last 10 transactions, show a
transaction in detail, transfer an asset to a whitelisted payee, and check the result by tx hash.

## Goals

- **T1 Keys.** Opens on the key list (the master's named subkeys), with the globally active key
  marked and each row showing its address. Lock state is shown once, in the status bar, because it
  belongs to the master and not to a key.
- **T2 Warm-up.** Enter opens a key *in the TUI* (an internal `--key`, not a global switch). If the
  wallet is locked, it asks for the password and unlocks for **1h** (`--warm <dur>`) through the
  staged unlock. Making the key globally active is a separate key press (`s`) with its own
  confirmation, because the active key is what the agent uses by default.
- **T3 Home.** The balance summary, plus the last 10 transactions for the current chain (§History).
- **T4 Tx detail.**
  - Fields: hash, from/to (nickname if listed), nonce, value and asset, fee
    (`gasUsed × effectiveGasPrice`), block, confirmations, status, explorer URL.
  - For this machine's sends it also shows the daemon-decoded action from the journal, labelled
    "local record".
- **T5 Assets.** Enter on the balance lists the native coin plus the registry tokens (USDT, USDC,
  DAI, WETH) with a non-zero balance.
- **T6 Transfer.** Enter on an asset starts a transfer:
  1. pick the recipient from the whitelist's **address entries valid on this chain**;
  2. enter the amount;
  3. review the send;
  4. confirm by typing the payee's nickname;
  5. the tx hash appears at once, and the screen follows its status (§Status).
- **T7 Whitelist.** List the entries (gen, `verified` state, policy), add an address entry or a
  contract entry with selectors, remove an entry, set the fee cap.
  - Every change asks for the password, even when warm, and runs the 04 edit protocol: the
    change and every outcome are shown above the password field, every daemon is stopped, and the
    wallet is left locked. The screen offers to warm up again (`--unlock <dur>`, the duration
    chosen on that screen, never the old expiry); the change shown says so before the password
    is typed.
  - The add screen shows the full EIP-55 address and never pre-fills an address from history
    (address-poisoning defence).
- **T8 Chain.** A chain switcher. The default is the last chain used, otherwise ethereum.

## Non-goals

- No mnemonic display and no backup screen.
- No free-form recipient field: no payee outside the whitelist, ever.
- No swaps, no `invoke` screen, no WalletConnect.
- No settings file.
- No second implementation: the TUI calls the same library code as the CLI. The send lock, the
  journal, the guards and the daemon whitelist all apply unchanged.

## Screens and keys

```
Keys ──Enter──▶ (password if locked) ──▶ Home [balance | recent tx] ──Enter on tx──▶ Tx detail
  │ s = make active (confirm)              ├─Enter on balance──▶ Assets ──Enter──▶ Transfer:
  │                                        │     payee (address entries) → amount → review → type nickname → status
  │                                        ├─ w ──▶ Whitelist (a add · d remove · f fee cap; password per change)
  │                                        └─ c ──▶ Chain picker
Global: ↑/↓ j/k · Enter · Esc back · r refresh · L lock (all daemons) · ? help · q quit
```

- **Status bar:** chain · key · short address · **`signing open (agent too) until 14:32`** or
  `locked` · RPC host (redacted). The warm-up opens signing for every client of the daemon,
  including the agent, and the bar says so.
- **Review screen:**
  - payee nickname and full address, asset and amount;
  - the fee estimate and the fee cap;
  - the whitelist verdict, the pending guard and the duplicate guard (02 §Send safety).
  - A guard failure shows its message. An override (allow-pending or allow-duplicate) is a
    separate key press, with the warning on screen.

## History (measured 2026-10-20)

Sources:
1. **The send journal.** This is a *local claim*. Every row from it is looked up on chain (`tx`)
   before it shows a status.
2. **A keyless explorer**, where one answers non-browser clients: `eth.blockscout.com` and
   `explorer.optimism.io`. Base, arbitrum and polygon Blockscout return a Cloudflare challenge
   (HTTP 403).

ERC-20 log scanning was dropped. Public nodes only serve about 5k blocks of logs (roughly 20 min on
arbitrum, about 3 h on base). Those logs are also where address-poisoning `Transfer(you,
lookalike, 0)` rows show up.

**Merge contract:**
- Rows are keyed by `(chainId, txHash)` and de-duplicated. One row can carry several token
  movements, and lists every source that reported it.
- Order: pending first, then block number descending, then by position in the block.
- The home view shows the first 10.
- A coverage note is always visible, for example "base: this machine's sends only; incoming
  transfers not indexed".

## Status of a send

States: `pending → mined (success | reverted)`. A tx becomes **`dropped/replaced`** when its nonce
has been mined by a different hash. When polling hits its bound, the screen says `unknown — run
zwallet tx`. Leaving the screen never cancels anything; the journal has already recorded the send.

## Password handling

- The terminal is already in raw mode, so there is no echo, before the field takes input.
- The password has one owner: a fixed-capacity `Zeroizing` buffer, rendered as `●`. Paste events
  are appended into that buffer, and then their event copy is dropped.
- No `Debug` or logging of any message that carries the password, and no event queue that keeps it.
- The buffer is cleared on success, cancel and every failure path. When a whitelist edit also
  warms up (`--unlock <dur>`), it is kept only until that new session has started, then
  cleared.
- A panic hook and normal unwinding both restore the terminal.

## Architecture

- `src/tui/` uses `ratatui` and `crossterm`. A pure state machine (`App` + `Msg` → `App`) is
  separate from rendering and I/O.
- RPC, explorer and daemon calls run as tokio tasks and post `Msg`s back, so every pane has
  loading and error states.
- Warm-up uses `protocol::start_daemon`, sends use the `prepare`/`sign_and_send` path, and the
  status display uses `tx` lookup.

## Acceptance

Each item is checked from real frames: tmux at 120×40, captured with `capture-pane`, and the
dispatcher reads the frames. Behaviour is checked against fixed fixtures on the mock RPC node from
`tests/evm_offline.rs`, with expected values written into the fixture, not computed by the CLI.

| ID | Run | Observe |
|---|---|---|
| T1 | 2 keys, `b` globally active | both listed, `b` marked; lock state only in the status bar |
| T2 | Enter on `a` (locked), type the password | bar shows `signing open (agent too) until` ≈ now+1h; `zwallet status` agrees; the global active key is still `b` |
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
| T9 | 80×24 resize; `q`; forced panic | layout fits; the terminal is restored after both |
| T10 | `zwallet lock` from another shell | the TUI shows `locked` within one refresh |
| T11 | pty test across T2 and T7 | no echo; the password appears in no captured frame; cancel mid-entry, then a retry, works |

**Tests:**
- `TestBackend` buffer assertions for each screen.
- `Msg`-sequence unit tests for the state machine.
- A pty end-to-end run of T2 → T6 → T7 against the mock node.
