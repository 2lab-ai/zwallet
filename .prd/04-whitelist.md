# zwallet — Signed whitelist (spend policy)

Status: **planned**, revised 2026-10-20 after two spec reviews (gpt6 and zhuge, both "change
before implementing"). It **ships in v0.1 as WU1b, before the v0.1.0 tag and before the wallet is
funded.** The v0.1 SSOT already fixed a daemon-enforced signing policy as v0.1 scope
(v0-1/ssot.md §Fixed scope), and this is that policy, shaped by the user's whitelist request
(v0-2-tui/ssot.md). The TUI's whitelist screen is v0.2 ([`03-tui.md`](03-tui.md)).

## Problem

Without a policy, an unlocked daemon signs any transaction it can decode, so a prompt-injected
agent can pay anyone for the whole unlock window (up to `1y`). The user asked (2026-10-20):
- a nickname → address list, stored under the wallet's password and **signed**, with a key
  separate from the seed;
- "anything not signed this way cannot be sent at all".

## What this guarantees (and what it does not)

**Guaranteed, within the threat model** (a same-user agent; see README §Known limits):
- **Native coin and registry tokens** leave this wallet only to **address entries** of a list
  that verifies against the sealed public key at the sealed generation, or to the signing key's
  own address.
- **Contract calls** go only to **contract entries**, and only with a listed selector.
- **Approvals** are revoke-only.
- **Fees** stay under the list's per-chain cap.

**Not guaranteed:**
- **Amounts.** There are no amount caps, so an agent can send the whole balance to any listed
  payee.
- **What a listed contract does.** A listed contract can forward value or act on arguments; a
  contract entry delegates that authority.
- **Joint rollback.** Restoring an older `wallet.json`, `whitelist.json` and `whitelist.key`
  together resurrects revoked entries. No same-user storage can stop that; a counter outside the
  home, such as the Secure Enclave, would be needed. The mitigation is visibility: `unlock` and
  `whitelist list` print the generation and the entries.
- **Independence from the password.** The separate key adds no second factor in v0.1: it is
  encrypted under the same password. What it buys is that the daemon never holds policy-signing
  authority, and that the key can later move to the Secure Enclave or to hardware.

## Design

**Files, all in the home and all 0600:**

| File | Contents | Who reads it |
|---|---|---|
| `wallet.json` (sealed payload v2) | mnemonic, `whitelistPub` (ed25519), `whitelistGen` | the daemon at unlock (pub and gen only, kept in memory); the editor |
| `whitelist.key` | the ed25519 secret, sealed under its own scrypt salt and nonce with the same password | the **editor only** (CLI/TUI with a typed password). Never the daemon |
| `whitelist.json` | `{version, gen, entries, policy, sig}`, plaintext and signed | everyone. Readable while locked; trusted only after verification |

**Entries.** Nicknames match `^[a-z0-9][a-z0-9_-]{0,31}$`, are unique, and each address appears
under one nickname.
- `{nickname, kind:"address", address, chains:[…]}`
  - The human may give `chains` as `["*"]` explicitly; the default is the one chain named or
    current.
- `{nickname, kind:"contract", address, chains:[one or more, never "*"], selectors:["0x…",…]}`
  - `selectors` must be non-empty, with no wildcard.
  - The address may not be a registry token.
  - Adding a contract entry prints the residual: "a listed contract may forward value or act on
    its arguments".

**Policy**, inside the signed list:
- `maxFeeWei` per chain: the cap on `gasLimit × maxFeePerGas`, or `× gasPrice` on legacy chains.
  Defaults are set at the first `add`: ethereum 0.003 ETH; base, arbitrum and optimism 0.0005
  ETH; polygon 2 POL; bsc 0.003 BNB; avalanche 0.05 AVAX; sepolia 0.01 ETH. Change with
  `whitelist fee-cap <chain> <amount>`.
- `allowMessageSigning` (default `false`). Some contracts accept EIP-191 signatures as spending
  authority, so `sign-message` is refused unless the human turns this on.

**Signature.** ed25519 over a canonical encoding of `{version, gen, entries, policy}`: sorted
keys, no whitespace, lower-cased addresses. The signature already binds the list to this wallet,
because only this wallet's sealed public key can verify it. There is no separate wallet id.

**Registry tokens.** These are built in: USDC, USDT, DAI and WETH per chain. The addresses are
pinned from the issuers' published lists, and a live test checks `symbol()`/`decimals()` on each
chain. Trust in their behavior is trust in the issuer, not something the code establishes. Only
registry tokens can be transferred. There is no user-added token kind in v0.1.

**Daemon enforcement** happens in `sign_tx`, on every request, after `decode_unsigned`:
1. Read `whitelist.json` with the safe reader. Verify `sig` with the in-memory `whitelistPub`, and
   check `gen == whitelistGen`. Any failure, including a missing list, is **`WHITELIST_INVALID`**:
   every value-moving signature is refused. A self-transfer is still allowed.
2. Check the fee: `gasLimit × max price ≤ policy.maxFeeWei[chain]`. Otherwise
   **`FEE_CAP_EXCEEDED`**.
3. Decide by the decoded action:

| Decoded action | Allowed only if |
|---|---|
| `native-transfer` | `to` is the signing key's own address, **or** an address entry valid on this chain |
| `transfer-call` | the token is a registry token, value is 0, `to` is an address entry valid on this chain, and `to` is not a token contract |
| `approve-call` | the token is a registry token, value is 0, and **amount == 0** (revoke). The spender doesn't matter |
| `transfer-from-call` | never |
| `contract-call` | `to` is a contract entry valid on this chain, and the selector is in its `selectors`. Native value is allowed only then |
| anything else (contract creation, other tx types) | never (already refused by the decoder) |

Otherwise the result is **`WHITELIST_DENIED`**. The error carries the counterparty and chain, and
the exact command the human runs to allow it:
`zwallet whitelist add <nick> <addr> --chains <chain>` (for a contract, plus `--selectors <sel>`).

**Edits.** These are `whitelist add | remove | fee-cap | allow-message-signing on|off`. Each runs
only with a typed password (TTY or `--password-stdin`) and is linearized like this:
1. Remember the current session's `expiresAt`, then **stop every daemon** (the `zwallet lock`
   path) and confirm none are left. Nothing can be signed while the list is changing.
2. Take `w.lock`, decrypt `wallet.json` and `whitelist.key`, and read the current list.
3. **Pick the base.** The current list is the base only if it verifies (sig with the sealed pub,
   `gen ==` sealed gen).
   - If it doesn't, and `whitelist.next.json` verifies at `gen == sealed gen`, promote that file:
     it is a committed-but-unrenamed edit.
   - If neither verifies, **never sign it.** Show its unverified entries and start from an empty
     list, so the human re-adds deliberately.
4. Apply the change and set `gen' = gen + 1`. Sign and write `whitelist.next.json`. Re-seal
   `wallet.json` with `whitelistGen = gen'` (temp, verify, rename). Then rename `next` to
   `whitelist.json`. A crash at any point is recovered by step 3: either the old generation stands,
   or the staged one is promoted.
5. Re-warm to the **remembered `expiresAt`** (not a fresh duration) with the password just typed,
   through the staged unlock. Then zeroize the password. If there was no session, stay locked.

The first `add` creates the key: it generates the ed25519 key and writes `whitelist.key`, and sets
`whitelistPub` and `whitelistGen = 1` in the seal. A v1 payload is upgraded to v2 at that moment.
`whitelist reset` (password) makes a new key and an empty list. It exists for when `whitelist.key`
is lost or corrupted, which can happen independently of `wallet.json`.

**CLI and dry-run.**
- `whitelist list` works while locked and prints `gen`, the entries and the policy, with
  `verified:false`. With `--verify` (password) it checks the signature locally and prints
  `verified:true`.
- `transfer --to` and `invoke --to` also accept `@nickname`.
- A dry-run checks the unverified list in the CLI first. If that check denies, the dry-run reports
  `whitelist:{allowed:false, reason}` and **does not ask the daemon to sign**, so there is no
  `recoveredFrom`. The daemon check stays unconditional for live sends.

**Invariants, pinned by tests.** zwallet never signs a raw hash, EIP-712 typed data, or an EIP-7702
authorization (type 4). Any of these would bypass the whitelist entirely. `decode_unsigned` accepts
only type-2 and legacy transactions, and `sign-message` is EIP-191 only and gated by
`allowMessageSigning`.

## Acceptance

| ID | Run | Observe |
|---|---|---|
| W1 | first `whitelist add alice 0x… --chains base` (password) | `whitelist.key` created; `list` (locked) shows alice, gen 1, `verified:false`; `--verify` gives `verified:true` |
| W2 | unlocked, no entry for X: `transfer --to X` | `WHITELIST_DENIED` with the exact add command; nothing signed; journal unchanged |
| W3 | `transfer --chain base --to @alice …` | signed and sent; `transfer --chain ethereum --to @alice` gives `WHITELIST_DENIED` (chain scope) |
| W4 | edit `whitelist.json` by hand, adding an address | every value send is `WHITELIST_INVALID`; the next password-backed edit refuses to sign the edited list, shows it, and starts from empty |
| W5 | swap `whitelist.json` alone for an older signed copy | `WHITELIST_INVALID` (gen mismatch) |
| W5b | restore an older `wallet.json`, `whitelist.json` and `whitelist.key` together | accepted, as documented. `unlock` prints the older gen and entries, so the human can see it |
| W6 | `approve(spender, 1)`; `transferFrom`; `invoke` of an unlisted contract or an unlisted selector; a token transfer to a token-contract address | each `WHITELIST_DENIED`; `approve(spender, 0)` is allowed |
| W7 | test-hook crash after each of the edit's three writes | the list is either the old gen or the promoted new gen; never invalid-and-signed |
| W8 | edit while the daemon is signing (concurrent) | the edit stops the daemon first; no signature lands after the edit starts; the session is re-warmed to the old `expiresAt` |
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
