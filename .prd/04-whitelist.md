# zwallet — Signed whitelist (spend policy)

Status: **in-progress** (WU1b, built 2026-10-20 in three parts: A core, B daemon enforcement, C
CLI and docs; this page is the as-built contract of all three). Planned after two spec reviews
(gpt6 and zhuge, both "change before implementing"). It **ships in v0.1, before the v0.1.0 tag
and before the wallet is funded.** The v0.1 SSOT already fixed a daemon-enforced signing policy
as v0.1 scope (v0-1/ssot.md §Fixed scope), and this is that policy, shaped by the user's
whitelist request (v0-2-tui/ssot.md). The TUI's whitelist screen is v0.2 ([`03-tui.md`](03-tui.md)).

## Problem

Without a policy, an unlocked daemon signs any transaction it can decode, so a prompt-injected
agent can pay anyone for the whole unlock window (up to `1y`). The user asked (2026-10-20):
- a nickname → address list, stored under the wallet's password and **signed**, with a key
  separate from the seed;
- "anything not signed this way cannot be sent at all".

## What this guarantees (and what it does not)

**Guaranteed, within the threat model** (a same-user agent; see README §Known limits):
- **Native coin and registry tokens** leave this wallet only to **address entries** of a list
  that verifies against the sealed anchor (public key, generation and digest), or as a native
  transfer to the signing key's own address (a token transfer to itself needs an entry too).
- **Contract calls** go only to **contract entries**, and only with a listed selector; never to a
  registry token.
- **Approvals** are revoke-only.
- **Fees** stay under a per-chain cap on every signature, a self-transfer included: the verified
  list's cap, or the built-in default while the wallet has no whitelist yet. A list that cannot
  be trusted refuses every signature, so it never relaxes a stricter signed cap.

**Not guaranteed:**
- **Amounts.** There are no amount caps, so an agent can send the whole balance to any listed
  payee.
- **What a listed contract does.** A listed contract can forward value or act on its arguments;
  a contract entry delegates that authority.
- **A total fee budget.** The fee cap bounds each transaction, not a sum: repeated transactions
  can each spend up to the cap.
- **Signing availability.** The daemon has at most 4 list reads in flight, and a request waits
  at most 2 s in all, for a free slot and for its read. A stalled file or too many requests at
  once make it "could not be read in time (2000 ms)", and every request meanwhile is refused
  (`WHITELIST_INVALID`). An agent can deny signing this way, never widen it.
- **The L1 data fee** on the OP-stack chains (Base, Optimism) is outside `gasLimit × maxFee`, and
  calldata can be up to about 512 KB.
- **Joint rollback.** The seal is authenticated (AES-GCM) but not fresh: an older `wallet.json`
  still opens with the password. Restoring an older `wallet.json`, `whitelist.json` and
  `whitelist.key` together therefore resurrects revoked entries. No same-user storage can stop
  that; a counter outside the home, such as the Secure Enclave, would be needed. The mitigation
  is visibility: `unlock`, `status` and `whitelist list` print the generation and the entries.
  Rolling back `wallet.json` alone fails closed: an older anchor does not match the current list,
  so every signature is `WHITELIST_INVALID`; a `wallet.json` from before the first `add` seals
  `{}`, no whitelist, so everything but a self-transfer under the built-in fee cap is refused.
- **Independence from the password.** The separate key adds no second factor in v0.1: it is
  encrypted under the same password. What it buys is that the daemon never holds policy-signing
  authority, and that the key can later move to the Secure Enclave or to hardware.

## Design

**Files, all in the home and all 0600:**

| File | Contents | Who reads it |
|---|---|---|
| `wallet.json` | the sealed payload `{version, mnemonic, policy}`; `policy` holds the anchor `{whitelistPub, whitelistGen, whitelistDigest}` (`{}` = no whitelist yet) | the daemon at unlock (the anchor only, kept in memory); the editor |
| `whitelist.key` | `{version: 1, crypto}`: the ed25519 secret in the same `CryptoBox` shape as wallet.json, its own salt and nonce, AAD `zwallet/whitelist.key/v1`, KDF parameters copied from wallet.json, same password | the **editor only** (CLI with a typed password). Never the daemon |
| `whitelist.json` | `{version, gen, entries, policy, sig}`, plaintext and signed | everyone. Readable while locked; trusted only after verification |
| `whitelist.next.json` | an edit staged between signing and publishing | the editor; the daemon only when the anchor names it (see **Edits**) |

**The anchor** sits inside the existing sealed `policy` object (no `SEALED_VERSION` bump): the
public key (hex, 32 bytes), the generation (a number), and the digest (keccak-256 of the signed
message, 32 bytes hex). The digest pins the one list signed at that generation, so an abandoned
edit's file signed at the same generation never verifies.

**Entries.** Nicknames match `^[a-z0-9][a-z0-9_-]{0,31}$`, are unique, and each address appears
under one nickname. Addresses and selectors are stored lower-case.
- `{nickname, kind:"address", address, chains:[…]}`
  - `chains` is `["*"]` only when the human gives `*`; the CLI default is `["ethereum"]`.
  - The address may be a registry token's (a token transfer *to* a token contract is still
    refused by the action table).
- `{nickname, kind:"contract", address, chains:[one or more, never "*"], selectors:["0x…",…]}`
  - `selectors` must be non-empty, with no wildcard.
  - The address may not be a registry token on **any** chain (its selectors would bypass the
    revoke-only approval rule).
  - Adding a contract entry prints the residual: "a listed contract may forward value or act on
    its arguments".

**Policy**, inside the signed list:
- `maxFeeWei` per chain key, a decimal string: the cap on `gasLimit × maxFeePerGas`, or
  `× gasPrice` on legacy chains. Defaults are set when a list is first made (first `add`, or
  `reset`): ethereum 0.003 ETH; base, arbitrum and optimism 0.0005 ETH; polygon 2 POL; bsc 0.003
  BNB; avalanche 0.05 AVAX; sepolia 0.01 ETH. Change with `whitelist fee-cap <chain> <amount>`
  (in the chain's coin, more than zero).
- `allowMessageSigning` (default `false`). Some contracts accept EIP-191 signatures as spending
  authority, so `sign-message` is refused unless the human turns this on.

**Signature.** ed25519 (strict verification) over `"zwallet/whitelist/v1\n"` followed by the
canonical encoding of `{version, gen, entries, policy}`: keys sorted, no whitespace, strings as
stored, wei amounts as decimal strings, `gen` and `version` as numbers. The encoder is written by
hand and pinned by a golden-bytes test. The signature binds the list to this wallet because only
this wallet's sealed public key verifies it; there is no separate wallet id. Verification does
not re-check the registry; the daemon's action table does. What each part guarantees:
- the **signature** says the list was made by whoever holds the key in `whitelist.key`: the
  editor, with the wallet password. The daemon only ever holds the public key, so it can check
  lists but never make one;
- the sealed **digest** says this is the one list committed at the sealed generation, so a list
  the key signed for an edit that never committed is not accepted in its place;
- the sealed **generation** names that commit for humans (`unlock`, `status` and `whitelist
  list` show it) and makes an older copy fail with a plain reason.

**Registry tokens.** Built in, 19 issuer-endorsed deployments of USDC, USDT, DAI and WETH, keyed by
(chainId, address), each with its source URL (`src/chains/evm/registry.rs`; Avalanche's Tether
reports the on-chain symbol `USDt`). A live test checks `symbol()` and `decimals()` of every row
on its chain. Trust in their behavior is trust in the issuer, not something the code establishes.
Only registry tokens can be transferred. There is no user-added token kind in v0.1.

**Daemon enforcement.** At unlock the daemon keeps the anchor from the unsealed policy. No
anchor, or one that does not parse, does not stop the unlock.
`unlock`, `create --unlock` and `status` show the session's view as `whitelist: {gen, verified:
true, entries: [nicknames]}`, or `{gen (null without an anchor), verified: false, entries: [],
reason}`; `status` reads the list again on every call. On every `sign_tx`, after
`decode_unsigned` and the chainId check, before `sign_decoded`:
1. Load the list: `whitelist.json` through the safe reader, verified against the anchor; if it
   does not verify, `whitelist.next.json` if that verifies at the sealed generation and digest
   (an edit that re-sealed and died before its rename). The read runs outside the session's
   lock, with at most 4 reads in flight; a request waits at most 2 s in all, for a free slot and
   for its read, and past that the list "could not be read in time (2000 ms)" and cannot be
   trusted. Before signing, the session, `wallet.json`'s ciphertext and, last, the expiry are
   checked again: a read that finishes after `lock`, expiry or a re-seal signs nothing.
2. The list's state decides what can be signed at all:
   - **verified** (`Usable`): steps 3 and 4;
   - **no whitelist yet** (`NoList`: wallet.json seals no anchor): only a native self-transfer,
     under the built-in default fee cap for the chain (`registry::DEFAULT_FEE_CAPS_WEI`; a chain
     without one is refused with the `fee-cap` command). Anything else is
     **`WHITELIST_INVALID`**, and its message and `command` carry the edit that would allow it,
     which creates the list: `zwallet whitelist add <nick> <addr> --chains <chain>`;
   - **cannot be trusted** (`Unusable`: it does not verify, cannot be read or not in time, or
     the sealed anchor is malformed): **every** signature, a native self-transfer included, is
     **`WHITELIST_INVALID`**. The message gives the reason and points to `zwallet whitelist list
     --verify`, with **no** `command` (an edit suggested from the refused transaction would name
     an address an agent chose, and rebuild the list around it). A broken list never relaxes a
     stricter signed fee cap.
3. Check the fee, on every signature: `gasLimit × (maxFeePerGas | gasPrice)`, in U256, against
   the verified list's `maxFeeWei[chain]` (or, with no whitelist yet, the built-in default).
   Above the cap, or no readable cap, is **`FEE_CAP_EXCEEDED`** naming `zwallet whitelist
   fee-cap <chain> <this transaction's fee in whole units>`, with `feeWei` (and `capWei` when
   there is a cap). Fee caps are never skipped, so a self-transfer cannot burn the balance in
   fees.
4. Decide by the decoded action:

| Decoded action | Allowed only if |
|---|---|
| `native-transfer` | `to` is the signing key's own address (any value, under the fee cap; with a verified list or no whitelist yet), **or** an address entry valid on this chain |
| `transfer-call` | the token is a registry token on this chain, value is 0, `to` is an address entry valid on this chain (the key's own address included), and `to` is not a registry token on any chain |
| `approve-call` | the token is a registry token on this chain, value is 0, and **amount == 0** (revoke). The spender doesn't matter |
| `transfer-from-call` | never |
| `contract-call` | `to` is not a registry token, `to` is a contract entry valid on this chain, and the selector is in its `selectors`. Native value is allowed only then |
| anything else (contract creation, other tx types) | never (already refused by the decoder) |

Otherwise the result is **`WHITELIST_DENIED`**. Every refusal carries `counterparty` and `chain`
in its details and, when one edit of the list allows it, the exact command in `command` and in
the message: `zwallet whitelist add <nick> <addr> --chains <chain>` for an address not in the
list (for a contract, plus `--contract --selectors <sel>`). `command` is left out (not null)
when no edit can allow it: `transferFrom`; an approve of more than 0; an approve on a token that
is not a registry token; any other call on a registry token; a token transfer to a token
contract; an address listed as the other kind. It is left out too when an entry exists but does
not cover the chain or the selector: the message names the entry and what it is missing, since
`whitelist add` refuses a listed nickname or address and the fix is a remove and an add, two
password-backed edits. `sign-message` is refused unless the verified list allows it:
**`WHITELIST_DENIED`** with `command: zwallet whitelist allow-message-signing on` while the
verified list has it off; **`WHITELIST_INVALID`** with that command while there is no whitelist;
**`WHITELIST_INVALID`** pointing to `list --verify`, with no command, while the list cannot be
trusted. `Response.details` is an optional
field of the daemon protocol, with no `PROTOCOL_VERSION` bump (the daemon and the CLI are always
the same binary).

**Edits.** `whitelist add | remove | fee-cap | allow-message-signing on|off | reset`. Each runs
only with a typed password (terminal prompt or `--password-stdin`), in this order:
0. Before the prompt: the arguments are checked as the list will check them (nickname, address,
   EIP-55 checksum, chains, selectors, no registry token as a contract, fee amount). With
   `--unlock <duration>`, `unlock`'s own start-up checks run too (the duration, Linux
   `ptrace_scope` (`--allow-ptrace-scope-0` accepts it), `/proc` hidepid) and the binary is
   pinned as `unlock` pins it. Then **the change is shown** on the channel the password is read
   from (the terminal with the prompt, stderr with `--password-stdin`): the verb, the nickname,
   the full EIP-55 address, payee or contract, the chains (`*` spelled out as every chain), the
   selectors, a policy change's old and new value, what a `reset` drops, and whitelist.json's
   generation and the next one. whitelist.json is unverified at that point (only the password
   opens the seal), so the text says that a list that is not the sealed one makes the change
   start from an empty list. Values from the file are escaped. An agent can start a change, but
   the human sees exactly what the password authorizes. After the prompt the password is checked
   against `wallet.json` and the change is tried on the list the edit would start from. A wrong
   password (`WRONG_PASSWORD`) or a refused change (an unknown nickname to remove, a nickname or
   address already listed) stops nothing.
1. **Stop every zwallet daemon of this user** (the `zwallet lock` path, all homes) and confirm
   none is left. Nothing can be signed while the list changes.
2. Take `w.lock`, decrypt `wallet.json` and `whitelist.key`. `w.lock` orders zwallet's own
   writers; it is a cooperative boundary, and a same-user process that ignores it can still
   write these files. What makes such a write visible is the signature and the seal, not the
   lock.
3. **Pick the base.** The current list is the base only if it verifies against the anchor.
   - If it doesn't, and `whitelist.next.json` does, promote that file (renamed into place before
     the change is staged): it is a committed but unrenamed edit.
   - If neither verifies, **never sign it.** The output shows it as `discarded` (the reason and
     its unverified entries as JSON), and the edit starts from an empty list with the default
     policy, so the human re-adds deliberately.
4. Apply the change at `gen' = gen + 1`, validate, sign, and write `whitelist.next.json`. Re-seal
   `wallet.json` with the new anchor (refused if its ciphertext changed since step 2). Then rename
   `next` to `whitelist.json`. A crash at any point is recovered by step 3 (and by the daemon's
   `next` fallback): either the old generation stands, or the staged one is promoted.
5. The wallet **stays locked**, unless `--unlock <duration>` was given: then a new session starts
   for that duration, the one typed in this command, with the password just typed, through the
   staged unlock. No duration or expiry is taken from the session that was running. The output
   shows the new `expiresAt` and the daemon's `whitelist` field. Then the password is wiped. If
   the edit fails after step 1, nothing is unlocked and the error says the wallet is locked.

The first `add` creates the key: it generates the ed25519 key, writes `whitelist.key`, and seals
the anchor at generation 1. `whitelist reset` makes a new key and an empty list with the default
policy at the next generation (old + 1), for when `whitelist.key` is lost or corrupted, which
can happen independently of `wallet.json`. A change's output: `{ change, gen, previousGen, base:
"current" | "promoted-next" | "empty", keyCreated, entries: [nicknames], verified: true,
stoppedDaemons, entry? | feeCap? | allowMessageSigning? | dropped?, discarded?, warning?, locked,
hint? }`, and with `--unlock` `{ duration, expiresAt, daemonPid, whitelist }` (or `unlockError`
if the new session could not start).

**CLI.**
- `whitelist list` works while locked and prints `gen`, the entries and the policy of
  `whitelist.json` as it is, with `verified: false`. Human output shows a value from the file as
  is only when it is short printable ASCII, else quoted with escapes and cut after 64
  characters; messages that echo file values are escaped by the core the same way. With
  `--verify` (password) it checks the list locally and prints `verified: true` with `source`
  (`whitelist.json`, or `whitelist.next.json` after an interrupted edit) and `sealedGen`, or
  `verified: false` with the `reason`.
- `transfer --to` and `invoke --to` also accept `@nickname`, resolved through `whitelist.json`
  as it is on disk (unverified; it only saves typing, the daemon decides). An unknown nickname
  is `VALIDATION_ERROR` naming `zwallet whitelist list`. The output adds `toNickname`.
- A dry-run (`transfer` or `invoke --dry-run`) checks the unverified list when there is one: it
  runs the daemon's own `check_tx` on the unsigned transaction against `whitelist.json` as it is
  on disk (`Usable` if it parses, `Unusable` with the reason if it cannot be read). If that
  refuses, the dry-run reports `whitelist: {allowed: false, reason, code, command?, counterparty,
  chain, …}` and **does not ask the daemon to sign**, so there is no `txHash` and no
  `recoveredFrom`. If it allows, the daemon checks against the list the seal names and signs, and
  the output adds `whitelist: {allowed: true}`. Without a `whitelist.json` the CLI checks nothing
  and the daemon decides, as for a live send: the CLI cannot tell a wallet with no whitelist yet
  from one whose list was deleted (the anchor is sealed), and a `NoList` refusal would suggest
  adding an address the caller chose. Its refusal is the usual error, and there is no
  `whitelist` field. The CLI never builds `NoList`. The daemon check stays unconditional for
  live sends.

**Invariants, pinned by tests.** zwallet never signs a raw hash, EIP-712 typed data, or an EIP-7702
authorization (type 4). Any of these would bypass the whitelist entirely. `decode_unsigned` accepts
only type-2 and legacy transactions, and `sign-message` is EIP-191 only and gated by
`allowMessageSigning`.

## As built: changes from the reviewed plan

From part A (core), accepted by the dispatcher:
1. The anchor is `policy.{whitelistPub, whitelistGen, whitelistDigest}` in the existing sealed
   payload; `{}` means no whitelist; no `SEALED_VERSION` bump (the plan said "sealed payload v2").
   The digest closes a same-generation crash hole.
2. The signed message is `"zwallet/whitelist/v1\n"` + the canonical bytes.
3. Contract entries may not sit at a registry token address on any chain.
4. Readers accept a `whitelist.next.json` that verifies at the sealed generation and digest (the
   re-seal → rename window); the daemon uses it too.
5. `reset` continues the generation (old + 1), and the new empty list gets the default policy.
6. Address entries at a registry token address are allowed (the daemon's "`to` is not a token
   contract" rule still applies to token transfers).
7. `whitelist.key` is `{version: 1, crypto: CryptoBox}` with AAD `zwallet/whitelist.key/v1` and
   wallet.json's KDF parameters.
8. Avalanche USDT's on-chain symbol is `USDt` (`Token::onchain_symbol`).

From part B (daemon), dispatcher decisions, with review round 1:
1. An unusable anchor does not stop `unlock`.
2. A `contract-call` to any registry token is denied whatever the entries say; a token transfer
   is refused to any address that is a registry token on any chain; an approve of 0 on a token
   outside the registry is denied; a token transfer to the key's own address needs a payee entry
   (only a native transfer to itself is exempt).
3. A missing or unreadable fee cap is `FEE_CAP_EXCEEDED`. Fee caps are never skipped: a native
   self-transfer is checked against the verified list's cap, or, while the wallet has no
   whitelist yet, the built-in default (the plan's "no list, no cap check" was a fund-loss path
   through fees). A list that cannot be trusted refuses every signature, self-transfers included
   (review round 2: a broken list must not relax a stricter signed cap).
4. `ListView` has three states: `Usable`, `NoList` (no anchor sealed) and `Unusable(reason)`.
   `WHITELIST_INVALID` names no edit and points to `zwallet whitelist list --verify` when the
   list cannot be trusted; only a wallet with no whitelist yet gets the first-add command.
5. An entry that does not cover the chain or the selector gets no `command` (the plan's
   `remove && add` was two edits, and a failed add would leave the entry gone).
6. The list is read outside the session's lock with at most 4 reads in flight and a 2 s wait in
   all (a free slot included; past it, "could not be read in time"), and the session, the
   ciphertext and, last, the expiry are checked again before signing.
7. `Response.details` carries the refusal details; no protocol version bump.

From part C (CLI), with review round 1:
1. The change is shown before the password is read, where the password is read from.
2. The password is checked, and the change tried on the current list, before any daemon is
   stopped (one more scrypt), so a typo or a refused change leaves a running session alone.
3. Step 1 stops every daemon of this user (the `lock` path), not only this home's.
4. No automatic re-warm: the plan re-warmed to the old session's `expiresAt`, which came from
   whatever answered for the home and became a fresh duration at start-up. A change leaves the
   wallet locked; `--unlock <duration>` starts a new session for a duration the human typed
   (`--allow-ptrace-scope-0` only with it).
5. `--chains` defaults to `ethereum`, the CLI's default chain (`--chain` is an alias); `fee-cap`
   refuses 0.
6. Human output escapes values read from `whitelist.json` (a hand-edited file could otherwise
   put terminal control sequences on the screen).
7. `create --unlock` shows the `whitelist` field as `unlock` does.
8. The dry-run checks the unverified list only when there is a `whitelist.json`; without one the
   daemon decides (the CLI cannot tell "no whitelist yet" from "deleted").

## Acceptance

| ID | Run | Observe |
|---|---|---|
| W1 | first `whitelist add alice 0x… --chains base` (password) | the change (address, chains, gen 1) is shown before the prompt; `whitelist.key` created; `list` (locked) shows alice, gen 1, `verified:false`; `--verify` gives `verified:true` |
| W2 | unlocked, no entry for X: `transfer --to X` | `WHITELIST_DENIED` with the exact add command; nothing signed; journal unchanged |
| W3 | `transfer --chain base --to @alice …` | signed and sent; `transfer --chain ethereum --to @alice` gives `WHITELIST_DENIED` (chain scope) |
| W4 | edit `whitelist.json` by hand, adding an address | every value send is `WHITELIST_INVALID`; the next password-backed edit refuses to sign the edited list, shows it as `discarded`, and starts from empty |
| W5 | swap `whitelist.json` alone for an older signed copy | `WHITELIST_INVALID` (gen mismatch) |
| W5b | restore an older `wallet.json`, `whitelist.json` and `whitelist.key` together | accepted, as documented. `unlock` prints the older gen and entries, so the human can see it |
| W6 | `approve(spender, 1)`; `transferFrom`; `invoke` of an unlisted contract or an unlisted selector; a token transfer to a token-contract address | each `WHITELIST_DENIED`; `approve(spender, 0)` is allowed |
| W7 | test-hook crash after each of the edit's three writes | the list is either the old gen or the promoted new gen; never invalid-and-signed; the next CLI edit continues from it |
| W8 | edit while unlocked (tested). A signature racing the edit is not run as a test: it relies on step 1 returning only once no daemon is left | a wrong password or a refused change stops nothing; otherwise every daemon is stopped first (other homes' too) and the wallet stays locked; with `--unlock 10m` a new session runs until now + 10 min and reports the new generation |
| W9 | fees above `maxFeeWei` (hostile RPC fixture, or a large `--max-fee-gwei`) | `FEE_CAP_EXCEEDED` from the daemon |
| W10 | `sign-message` with `allowMessageSigning` off, then on | refused, then signed |
| W11 | 0-value self-transfer dry-run and live (A7), no list | allowed (under the built-in default fee cap); with a list that cannot be trusted, refused (`WHITELIST_INVALID`) |
| W12 | invariant tests | type-4, typed-data and raw-hash requests have no protocol path and are refused |
| W13 | `transfer --dry-run` to an address the list refuses | `whitelist: {allowed: false, …}` with the add command, and no signature request reaches the daemon; an allowed dry-run is signed and shows `allowed: true`; without a `whitelist.json` the daemon is asked and the output has no `whitelist` field |

## Not in v0.1 (stays planned)

- Amount caps (a rolling 24 h native cap, per-token caps).
- The `invoke` duplicate key on calldata, and the duplicate window as a policy setting (R4 N-3 and
  N-4).
- User-added tokens.
- Moving the whitelist key to the Secure Enclave or hardware.
