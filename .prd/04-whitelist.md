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
  transfer to the signing key's own address.
- **Contract calls** go only to **contract entries**, and only with a listed selector; never to a
  registry token.
- **Approvals** are revoke-only.
- **Fees** stay under the list's per-chain cap.

**Not guaranteed:**
- **Amounts.** There are no amount caps, so an agent can send the whole balance to any listed
  payee.
- **What a listed contract does.** A listed contract can forward value or act on its arguments;
  a contract entry delegates that authority.
- **Joint rollback.** The seal is authenticated (AES-GCM) but not fresh: an older `wallet.json`
  still opens with the password. Restoring an older `wallet.json`, `whitelist.json` and
  `whitelist.key` together therefore resurrects revoked entries. No same-user storage can stop
  that; a counter outside the home, such as the Secure Enclave, would be needed. The mitigation
  is visibility: `unlock`, `status` and `whitelist list` print the generation and the entries.
  Rolling back `wallet.json` alone fails closed: an older anchor does not match the current list
  (`WHITELIST_INVALID`), and a `wallet.json` from before the first `add` seals `{}`, no
  whitelist, so every value-moving signature is refused.
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
anchor, or one that does not parse, does not stop the unlock: the session's list is unusable
(`unlock` and `status` show `whitelist: {verified: false, reason}`). On every `sign_tx`, after
`decode_unsigned` and the chainId check, before `sign_decoded`:
1. Load the list: `whitelist.json` through the safe reader, verified against the anchor; if it
   does not verify, `whitelist.next.json` if that verifies at the sealed generation and digest
   (an edit that re-sealed and died before its rename). Anything else, including no list, is
   **`WHITELIST_INVALID`** for every value-moving signature; with no anchor the message carries
   the first-add command `zwallet whitelist add <nick> <addr> --chains <chain>`. A native
   self-transfer is still allowed.
2. Check the fee: `gasLimit × (maxFeePerGas | gasPrice) ≤ policy.maxFeeWei[chain]`, in U256. A
   higher fee, or no readable cap for the chain, is **`FEE_CAP_EXCEEDED`** naming
   `zwallet whitelist fee-cap <chain> <amount>`.
3. Decide by the decoded action:

| Decoded action | Allowed only if |
|---|---|
| `native-transfer` | `to` is the signing key's own address (any value, any list state; under the fee cap when the list is usable), **or** an address entry valid on this chain |
| `transfer-call` | the token is a registry token on this chain, value is 0, `to` is an address entry valid on this chain, and `to` is not a token contract |
| `approve-call` | the token is a registry token on this chain, value is 0, and **amount == 0** (revoke). The spender doesn't matter |
| `transfer-from-call` | never |
| `contract-call` | `to` is not a registry token, `to` is a contract entry valid on this chain, and the selector is in its `selectors`. Native value is allowed only then |
| anything else (contract creation, other tx types) | never (already refused by the decoder) |

Otherwise the result is **`WHITELIST_DENIED`**. Each of the three errors carries `counterparty`,
`chain` and `command` in its details, and the exact command the human runs in its message, e.g.
`zwallet whitelist add <nick> <addr> --chains <chain>` (for a contract, plus `--contract
--selectors <sel>`). `sign-message` is **`WHITELIST_DENIED`** naming
`zwallet whitelist allow-message-signing on` unless the verified list allows it. `unlock` and
`status` show the session's view as `whitelist: {gen, verified, entries: [nicknames], reason?}`.

**Edits.** `whitelist add | remove | fee-cap | allow-message-signing on|off | reset`. Each runs
only with a typed password (terminal prompt or `--password-stdin`), in this order:
0. Before the prompt: the arguments are checked as the list will check them (nickname, address,
   EIP-55 checksum, chains, selectors, no registry token as a contract, fee amount), and the
   binary that may re-warm is pinned (as `unlock` pins it). If a session is running, `unlock`'s
   own start-up checks run here too (Linux `ptrace_scope`, `/proc` hidepid;
   `--allow-ptrace-scope-0` accepts the first). After the prompt the password is checked against
   `wallet.json` and the change is tried on the list the edit would start from. A wrong password
   (`WRONG_PASSWORD`) or a refused change (an unknown nickname to remove, a nickname or address
   already listed) stops nothing.
1. Remember the current session's `expiresAt`, then **stop every zwallet daemon of this user**
   (the `zwallet lock` path, all homes) and confirm none is left. Nothing can be signed while the
   list changes.
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
5. Re-warm to the **remembered `expiresAt`** (not a fresh duration): unlock again with the
   password just typed, through the staged unlock, for `expiresAt − now` whole seconds, so the
   new session ends at the old time plus the new daemon's start-up. Then the password is wiped.
   If there was no session, or it ran out meanwhile, the wallet stays locked. Only this home's
   session is re-warmed; daemons of other homes stopped in step 1 stay stopped. If the edit
   fails after step 1, the re-warm still runs and the error says whether the session is back.

The first `add` creates the key: it generates the ed25519 key, writes `whitelist.key`, and seals
the anchor at generation 1. `whitelist reset` makes a new key and an empty list with the default
policy at the next generation (old + 1), for when `whitelist.key` is lost or corrupted, which
can happen independently of `wallet.json`. A change's output: `{ change, gen, previousGen, base:
"current" | "promoted-next" | "empty", keyCreated, entries: [nicknames], verified: true,
stoppedDaemons, entry? | feeCap? | allowMessageSigning? | dropped?, discarded?, warning?, locked,
previousExpiresAt?, expiresAt?, daemonPid?, rewarm? }`.

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
- **Not yet wired** (follow-up after parts B and C merge): a dry-run first checks the unverified
  list in the CLI with the daemon's own `check_tx`. If that denies, the dry-run reports
  `whitelist: {allowed: false, reason}` and **does not ask the daemon to sign**, so there is no
  `recoveredFrom`. The daemon check stays unconditional for live sends.

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

From part B (daemon), dispatcher decisions: an unusable anchor does not stop `unlock`; a
`contract-call` to any registry token is denied whatever the entries say; a missing or
unreadable fee cap is `FEE_CAP_EXCEEDED`; a native self-transfer is allowed in any list state
and checked against the fee cap only when the list is usable (so a fresh wallet's 0-value
self-transfer works).

From part C (CLI):
1. The password is checked, and the change tried on the current list, before any daemon is
   stopped (one more scrypt), so a typo or a refused change leaves a running session alone.
2. Step 1 stops every daemon of this user (the `lock` path), not only this home's; only this
   home's session is re-warmed.
3. Change verbs take `--allow-ptrace-scope-0` (the re-warm is an unlock); the checks run only
   when a session is running.
4. `--chains` defaults to `ethereum`, the CLI's default chain (`--chain` is an alias); `fee-cap`
   refuses 0.
5. The dry-run pre-check is not wired yet (see **CLI**).
6. Human output escapes values read from `whitelist.json` (a hand-edited file could otherwise
   put terminal control sequences on the screen).

## Acceptance

| ID | Run | Observe |
|---|---|---|
| W1 | first `whitelist add alice 0x… --chains base` (password) | `whitelist.key` created; `list` (locked) shows alice, gen 1, `verified:false`; `--verify` gives `verified:true` |
| W2 | unlocked, no entry for X: `transfer --to X` | `WHITELIST_DENIED` with the exact add command; nothing signed; journal unchanged |
| W3 | `transfer --chain base --to @alice …` | signed and sent; `transfer --chain ethereum --to @alice` gives `WHITELIST_DENIED` (chain scope) |
| W4 | edit `whitelist.json` by hand, adding an address | every value send is `WHITELIST_INVALID`; the next password-backed edit refuses to sign the edited list, shows it as `discarded`, and starts from empty |
| W5 | swap `whitelist.json` alone for an older signed copy | `WHITELIST_INVALID` (gen mismatch) |
| W5b | restore an older `wallet.json`, `whitelist.json` and `whitelist.key` together | accepted, as documented. `unlock` prints the older gen and entries, so the human can see it |
| W6 | `approve(spender, 1)`; `transferFrom`; `invoke` of an unlisted contract or an unlisted selector; a token transfer to a token-contract address | each `WHITELIST_DENIED`; `approve(spender, 0)` is allowed |
| W7 | test-hook crash after each of the edit's three writes | the list is either the old gen or the promoted new gen; never invalid-and-signed; the next CLI edit continues from it |
| W8 | edit while the daemon is signing (concurrent) | the edit stops the daemon first; no signature lands after the edit starts; the session is re-warmed to the old `expiresAt` (plus the new daemon's start-up) |
| W9 | fees above `maxFeeWei` (hostile RPC fixture, or a large `--max-fee-gwei`) | `FEE_CAP_EXCEEDED` from the daemon |
| W10 | `sign-message` with `allowMessageSigning` off, then on | refused, then signed |
| W11 | 0-value self-transfer dry-run and live (A7), no list | allowed |
| W12 | invariant tests | type-4, typed-data and raw-hash requests have no protocol path and are refused |

## Not in v0.1 (stays planned)

- Amount caps (a rolling 24 h native cap, per-token caps).
- The `invoke` duplicate key on calldata, and the duplicate window as a policy setting (R4 N-3 and
  N-4).
- User-added tokens.
- Moving the whitelist key to the Secure Enclave or hardware.
