# zwallet — Signed whitelist (spend policy)

Status: **shipped** (WU1b, 2026-10-21, PR #4; built 2026-10-20 in three parts: A core, B daemon
enforcement, C CLI and docs). This page is the as-built contract of all three, not a future
target. Planned after two spec reviews (gpt6 and zhuge, both "change before implementing"); it
ships in v0.1, before the v0.1.0 tag and before the wallet is funded. The v0.1 SSOT fixed a
daemon-enforced signing policy as v0.1 scope (v0-1/ssot.md §Fixed scope), and this is that
policy, shaped by the user's whitelist request (v0-2-tui/ssot.md). The TUI's whitelist screen is
v0.2 ([`03-tui.md`](03-tui.md)). Receipt: [`v0-1/loop.done.md`](v0-1/loop.done.md) R4.

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
- **ERC-20 `transfer`, `approve`, `transferFrom`** are judged as such whatever the entries say:
  only registry tokens are transferred, approvals are revoke-only, `transferFrom` is never
  signed.
- **Other contract calls** go only to **contract entries**, and only with a listed selector;
  never to a registry token.
- **Fees.** Each transaction, a native self-transfer included, must pass the execution-fee cap
  for its chain (`gasLimit × max price`): the verified list's, or the built-in default before
  any whitelist exists. A list that cannot be trusted refuses every signature, so it never
  relaxes a stricter signed cap. The cap is per transaction (see below).

**Not guaranteed:**
- **Amounts.** There are no amount caps, so an agent can send the whole balance to any listed
  payee.
- **What a listed contract does.** A listed contract can forward value or act on its arguments;
  a contract entry delegates that authority. That includes a token outside the registry listed
  as a contract: its listed selectors (other than the decoded `transfer`/`approve`/
  `transferFrom`, which stay refused) are called like any listed contract's.
- **A total fee budget.** The fee cap bounds each transaction, not a sum: repeated transactions
  can each spend up to the cap.
- **Signing availability.** The daemon has at most 4 list reads in flight. A signing request
  whose list read cannot complete within the 2 s deadline (waiting for a free slot counts
  against it) is refused (`WHITELIST_INVALID`, "could not be read in time (2000 ms)"). A stalled
  file or a flood of requests can deny signing this way, never widen it.
- **The L1 data fee** on the OP-stack chains (Base, Optimism) is outside `gasLimit × maxFee`, and
  calldata can be up to about 512 KB.
- **Joint rollback.** The seal is authenticated (AES-GCM) but not fresh: an older `wallet.json`
  still opens with the password. Restoring an older `wallet.json` and `whitelist.json` together
  therefore resurrects revoked entries. (The older `wallet.json` seals its own salt, so the same
  password derives the key it names.) No same-user storage can stop that; a counter outside the
  home, such as the Secure Enclave, would be needed. The mitigation is visibility: `unlock`,
  `status` and `whitelist list` print the generation and the entries.
  Rolling back `wallet.json` alone to an older edit fails closed: its anchor does not match the
  current list, so every signature is `WHITELIST_INVALID`. The reason names the cause, by the
  `whitelist.json` it finds above the seal (gen N, the seal at gen M):
  - signed by the sealed key, N ≥ M + 2: "whitelist.json (gen N) is newer than the seal (gen
    M): wallet.json was rolled back";
  - signed by the sealed key, N = M + 1: "... is one edit newer than the seal ..., under this
    wallet's whitelist key: an edit that never finished sealing, or a rolled-back wallet.json"
    (an edit whose re-seal failed leaves a staged copy that, in place of `whitelist.json`, looks
    the same);
  - signed by another key: "... is newer than the seal ... but not signed by this wallet's
    whitelist key: a wallet.json rolled back across a reset, a reset that never finished
    sealing, or a foreign file".

  Whatever the cause, the edit refusals and `list --verify` give one recovery: "If wallet.json
  is an older copy, restore the current one if you have it (zwallet keeps no copy of it);
  otherwise `zwallet whitelist reset` starts over ...". zwallet keeps no copy of `wallet.json`
  (`zwallet backup` prints only the recovery phrase), so restoring the current one needs a copy
  the human made after the last edit.
- **zwallet never suggests restoring an older whitelist.json.** A copy restored by hand that
  verifies against the sealed digest is the older list, so restoring one can complete a
  rollback (W5b). zwallet keeps no counter outside the home, so no condition it could state
  tells such a copy apart from the current list. Recovering from accidental damage to the list
  (deleted, edited, replaced) is therefore restoring the current `wallet.json` if that is what
  went back, or `whitelist reset`: the entries are added again by hand, and the summary states
  the cost (every fee cap back to the default, no payee and no contract) before the password.
- **Rollback to before the first edit.** A `wallet.json` from before the first `add` seals no
  whitelist, so the daemon sees "no whitelist yet": no payee send and no contract call is
  signed, but native self-transfers are, under the **built-in default caps**, which may be looser
  than the caps the human signed. `unlock` and `status` show `whitelist: {gen: null, verified:
  false}`, which says so. Every edit refuses in that state, because a list file exists (step 3):
  put the current `wallet.json` back (a copy made after the last edit), or `whitelist reset`
  (every fee cap back to the default, no payee and no contract).
- **Independence from the password.** The separate key adds no second factor in v0.1: it is
  derived from the same password. What it buys is that the daemon never holds policy-signing
  authority (a later version can move the key to the Secure Enclave or to hardware). Any change
  to the password or to wallet.json's KDF parameters (zwallet has no command for either yet) must
  sign the list again under the key they then derive; until it does, every edit refuses the key.

## Design

**Files, all in the home and all 0600:**

| File | Contents | Who reads it |
|---|---|---|
| `wallet.json` | the sealed payload `{version, mnemonic, policy}`; `policy` holds the anchor `{whitelistPub, whitelistGen, whitelistDigest}` and the key's salt `whitelistSalt` (`{}` = no whitelist yet) | the daemon at unlock (keeps the anchor only, in memory; drops the salt); the editor (anchor and salt) |
| `whitelist.json` | `{version, gen, entries, policy, sig}`, plaintext and signed | everyone. Readable while locked; trusted only after verification |
| `whitelist.next.json` | an edit staged between signing and publishing | the editor; the daemon only when the anchor names it (see **Edits**) |

There is no key file. A `whitelist.key` left by zwallet 0.1.0 is not read and not removed.

**The anchor** sits inside the existing sealed `policy` object (no `SEALED_VERSION` bump): the
public key (hex, 32 bytes), the generation (a number), and the digest (keccak-256 of the signed
message, 32 bytes hex). The digest pins the one list signed at that generation, so an abandoned
edit's file signed at the same generation never verifies. The generation and the digest are
what the public key alone cannot give: without them an older list the same key signed would
verify too.

**The key** is derived, never stored. `whitelistSalt`, next to the anchor in the sealed policy, is
32 random bytes (lower-case hex) drawn by the first edit and by every reset. Each edit derives the
ed25519 secret as scrypt(password, `"zwallet/whitelist-key/v1\n"` ‖ salt) with the scrypt parameters
that opened `wallet.json` (validated as wallet.json's are, never re-read from disk), then checks
that its public key is the sealed one. It is a scrypt run of its own, not derived from the AES key
that opens `wallet.json`, so holding that key never yields the whitelist key; an edit costs one
scrypt to open `wallet.json` and one to derive the key, as decrypting the 0.1.0 key file did.
`wallet.json`'s own envelope salt must be exactly 32 bytes (the only length any zwallet has drawn),
so its AES key is never scrypt over this domain-prefixed input. The daemon keeps only the three
anchor keys: at unlock it decrypts the whole policy, checks that the salt is there and well-formed,
and drops it with the rest once the session starts; its session never holds the salt or any of the
key. An anchor without a salt was sealed by zwallet 0.1.0 (or comes from a `wallet.json` restored
from before the upgrade): the daemon treats it as untrusted (`Unusable`, never `NoList`) with the
cause alone as its reason ("this whitelist was made by zwallet 0.1.0, whose key file is no longer
used"), as every daemon reason names only a cause. `list --verify` and every edit, which refuses
before anything is stopped, add the other cause and the one recovery: "... (a wallet zwallet 0.1.0
made, or a wallet.json restored from before the upgrade). If wallet.json is an older copy, restore
the current one ...; otherwise `zwallet whitelist reset` starts over ...". `reset` starts over under
a new salt. The seed is not derived this way: it stays a random BIP39 mnemonic encrypted under the
password, because a seed derived from the password would be a "brain wallet", open to password
guessing against its public addresses.

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
- the **signature** says the list was made by whoever holds the wallet password, from which the
  editor derives the key. The daemon only ever holds the public key, so it can check lists but
  never make one;
- the sealed **digest** says this is the one list committed at the sealed generation, so a list
  the key signed for an edit that never committed is not accepted in its place;
- the sealed **generation** names that commit for humans (`unlock`, `status` and `whitelist
  list` show it) and makes an older copy fail with a plain reason.

**Registry tokens.** Built in, 19 issuer-endorsed deployments of USDC, USDT, DAI and WETH, keyed by
(chainId, address), each with its source URL (`src/chains/evm/registry.rs`; Avalanche's Tether
reports the on-chain symbol `USDt`). A live test checks `symbol()` and `decimals()` of every row
on its chain. Trust in their behavior is trust in the issuer, not something the code establishes.
Only registry tokens can be transferred. There is no user-added token kind in v0.1.

**Daemon enforcement.** At unlock the daemon keeps the anchor (public key, generation, digest)
from the unsealed policy, and nothing else of it. No anchor, or one that does not parse or has no
salt (zwallet 0.1.0), does not stop the unlock.
`unlock`, `create --unlock` and `status` show the session's view as `whitelist: {gen, verified:
true, entries: [nicknames]}`, or `{gen (null without an anchor), verified: false, entries: [],
reason}`; `status` reads the list again on every call. On every `sign_tx`, after
`decode_unsigned` and the chainId check, before `sign_decoded`:
1. Load the list: `whitelist.json` through the safe reader, verified against the anchor; if it
   does not verify, `whitelist.next.json` if that verifies at the sealed generation and digest
   (an edit that re-sealed and died before it wrote `whitelist.json`). The read runs outside
   the session's lock, with at most 4 reads in flight; a request waits at most 2 s in all, for a
   free slot and for its read, and past that the list "could not be read in time (2000 ms)" and
   cannot be trusted. Before signing, the session, `wallet.json`'s ciphertext and, last, the
   expiry are checked again: a read that finishes after `lock`, expiry or a re-seal signs
   nothing.
2. The list's state decides what can be signed at all:
   - **verified** (`Usable`): step 3 (the action), then step 4 (the fee); a native transfer to
     the signing key's own address skips step 3;
   - **no whitelist yet** (`NoList`: wallet.json seals no anchor): only a native self-transfer,
     under the built-in default fee cap for the chain (`registry::DEFAULT_FEE_CAPS_WEI`; a chain
     without one is refused with the `fee-cap` command): step 4 only. Anything else is
     **`WHITELIST_INVALID`**; when an entry could allow the action, its message and `command`
     carry the add command that would allow it, and which creates the list (`zwallet whitelist
     add <nick> <addr> --chains <chain>`); a forbidden action has none, and neither has a revoke of
     a registry token (see **What a refusal carries**);
   - **cannot be trusted** (`Unusable`: it does not verify, cannot be read or not in time, or
     the sealed anchor is malformed or was sealed by zwallet 0.1.0): **every** signature, a
     native self-transfer included, is **`WHITELIST_INVALID`**. The message gives the reason and
     points to `zwallet whitelist list --verify`, with **no** `command` (an edit suggested from
     the refused transaction would name an address an agent chose, and rebuild the list around
     it). A broken list never relaxes a stricter signed fee cap. A `whitelist.json` above the
     sealed generation gets a reason by its generation and signer: a rolled-back
     `wallet.json`, an edit or reset that never finished sealing, a rollback across a reset, or
     a foreign file (see **Joint rollback**).
3. Decide by the decoded action:

| Decoded action | Allowed only if |
|---|---|
| `native-transfer` | `to` is the signing key's own address (any value, under the fee cap; with a verified list or no whitelist yet), **or** an address entry valid on this chain |
| `transfer-call` | the token is a registry token on this chain, value is 0, `to` is an address entry valid on this chain (the key's own address included), and `to` is not a registry token on any chain |
| `approve-call` | the token is a registry token on this chain, value is 0, and **amount == 0** (revoke). The spender doesn't matter |
| `transfer-from-call` | never |
| `contract-call` | `to` is not a registry token, `to` is a contract entry valid on this chain, and the selector is in its `selectors`. Native value is allowed only then |
| anything else (contract creation, other tx types) | never (already refused by the decoder) |

Otherwise the result is **`WHITELIST_DENIED`**. The `transfer-call`, `approve-call` and
`transfer-from-call` rows apply to any token, whatever the entries say (a token outside the
registry is refused there); every other selector is a `contract-call`.

4. Check the fee, on every signature that gets this far: `gasLimit × (maxFeePerGas |
   gasPrice)`, in U256, against the verified list's `maxFeeWei[chain]` (or, with no whitelist
   yet, the built-in default). Above the cap, or no readable cap, is **`FEE_CAP_EXCEEDED`**
   naming `zwallet whitelist fee-cap <chain> <this transaction's fee in whole units>`, with
   `feeWei` (and `capWei` when there is a cap). Fee caps are never skipped: each native
   self-transfer must pass the execution-fee cap for its chain (the verified list's, or the
   built-in default before any whitelist exists). The cap is per transaction, so repeated
   transactions can each spend up to it, and on the OP-stack chains the L1 data fee is charged
   on top of it. With a verified list the action comes first, so a fee refusal and its
   `fee-cap` command only ever concern a transaction the list would otherwise sign: a forbidden
   action with a huge fee is `WHITELIST_DENIED`, never a cap raise that allows nothing. With no
   whitelist yet, anything but a native self-transfer is `WHITELIST_INVALID` before any fee
   check (step 2).

**What a refusal carries.**
- A transaction refusal (`WHITELIST_DENIED`, `WHITELIST_INVALID`, `FEE_CAP_EXCEEDED`) carries
  `counterparty` and `chain` in its details and, when one edit of the list allows what was
  refused, the exact command in `command` and in the message: `zwallet whitelist add <nick>
  <addr> --chains <chain>` for an address not in the list (for a contract, plus `--contract
  --selectors <sel>`), or `zwallet whitelist fee-cap <chain> <fee>`; a fee refusal also carries
  `feeWei`, and `capWei` when there is a cap. A `fee-cap` command comes only with an action the
  list allows (the action is decided first), so it is the one edit that lets the request
  through. An add command allows the action; the fee is then checked against the cap, and a
  request over the cap too is refused next with its own `fee-cap` command.
- `command` is left out (not null) when no edit can allow it: `transferFrom`; an approve of more
  than 0; an approve on a token that is not a registry token; any other call on a registry
  token; a token transfer to a token contract; an address listed as the other kind. It is left
  out too when an entry exists but does not cover the chain or the selector (the message names
  the entry and what it is missing, since `whitelist add` refuses a listed nickname or address
  and the fix is a remove and an add, two password-backed edits), and for every refusal of a list
  that cannot be trusted.
- With no whitelist yet (`NoList`), `command` is the add command only when an entry could allow
  the action; a forbidden action has none, and neither has a revoke of a registry token (any
  signed list allows it).
- A `sign-message` refusal carries no `counterparty` or `chain`: **`WHITELIST_DENIED`** with
  `command: zwallet whitelist allow-message-signing on` while the verified list has it off;
  **`WHITELIST_INVALID`** with that `command` while there is no whitelist; **`WHITELIST_INVALID`**
  with no `command`, pointing to `list --verify`, while the list cannot be trusted.

`Response.details` is an optional field of the daemon protocol, with no `PROTOCOL_VERSION` bump
(the daemon and the CLI are always the same binary).

**Edits.** `whitelist add | remove | fee-cap | allow-message-signing on|off | reset`. Each runs
only with a typed password (terminal prompt or `--password-stdin`), in this order:
0. Before the prompt: the arguments are checked as the list will check them (nickname, address,
   EIP-55 checksum, chains, selectors, no registry token as a contract, fee amount; planned
   2026-10-28 for TUI-6: no entry at the zero address, refused here and not at parse,
   [`03-tui.md`](03-tui.md) §Whitelist edits step 0). With
   `--unlock <duration>`, `unlock`'s own start-up checks run too (the duration, Linux
   `ptrace_scope` (`--allow-ptrace-scope-0` accepts it), `/proc` hidepid) and the binary is
   pinned as `unlock` pins it. Then `whitelist.json` and `whitelist.next.json` are **read once**
   and **the change is shown** on the channel the password is read from (the terminal with the
   prompt, stderr with `--password-stdin`):
   - the verb, the nickname, the full EIP-55 address, payee or contract, the chains (`*` spelled
     out as every chain), the selectors; a policy change's old and new value, from both files
     when an unfinished edit is in `whitelist.next.json` ("whitelist.json says X;
     whitelist.next.json says Y -> Z"); for a `reset`, the entries it drops, every fee cap it
     sets back to the built-in default next to what the files say (a chain with a cap in the
     list but no built-in default: "no default: every send there is refused until `fee-cap`
     sets one"), message signing, that it lists no payee and no contract, and the one
     alternative: "if wallet.json is an older copy, restore the current one instead if you have
     it (zwallet keeps no copy of it)";
   - both files as read (generation and entries, unverified; name lists cut at 20). A file that
     exists but cannot be read or parsed is never shown as "none": "whitelist.json is there but
     cannot be read (<reason>): its entries are unknown, and a reset drops them" (the same for
     `whitelist.next.json`);
   - every outcome that can still happen, decided once the password opens `wallet.json`. For an
     edit: the change lands on `whitelist.json` (gen +1) if it is the sealed list; on the
     unfinished edit (its gen +1) if that is; with no list file at all, a first list from the
     built-in defaults with a new key (gen 1) if `wallet.json` seals no whitelist yet; otherwise
     nothing changes. For a `reset`, the generation it makes in each case: `whitelist.json`'s gen
     + 1 if that is the sealed list; `whitelist.next.json`'s gen + 1 if that is; "if neither
     file is the sealed list (it was changed or deleted): gen M+1, where M is the sealed
     generation, shown once the password opens wallet.json"; gen 1 if `wallet.json` seals no
     whitelist yet;
   - that every daemon of this user is stopped, and what happens to signing: "after the change:
     the wallet stays locked", or "after the change: a new signing session for <dur> (until
     about <local time>, <UTC>)";
   - last, right above the prompt, the change in one line with what happens to signing.

   Values from the files are escaped. After the prompt the password is checked against
   `wallet.json`, the base is picked from the bytes read (step 3's rules), and the change is
   tried on it. A wrong password (`WRONG_PASSWORD`) stops nothing; neither does a list shown
   that an edit cannot build on (an edit; a `reset` goes ahead), or a refused change (an unknown
   nickname to remove, a nickname or address already listed). Right before step 1 the files are
   read again and compared with what was shown: a swap after the summary refuses here, with
   nothing stopped.
1. **Stop every zwallet daemon of this user** (the `zwallet lock` path, all homes) and confirm
   none is left. Nothing can be signed while the list changes.
2. Take `w.lock` and decrypt `wallet.json`. `w.lock` orders zwallet's own
   writers; it is a cooperative boundary, and a same-user process that ignores it can still
   write these files. What makes such a write visible is the signature and the seal, not the
   lock.
3. **Read the list files once, bound to what was shown, and pick the base.** `whitelist.json`
   and `whitelist.next.json` are read once more, under `w.lock`, and every decision below is made
   from those bytes. If either is not the bytes shown in step 0 (a fingerprint of each, or
   absent), the change refuses before writing anything: "<file> changed after it was shown;
   nothing in the whitelist changed". So **the change is applied to exactly the list shown, or
   nothing changes.** The current list is the base only if it verifies against the anchor.
   The key is derived after the base is picked (an edit from the sealed salt; the first edit and
   a reset draw a new one), so what is wrong with the list, a rollback across a reset say, is
   named before a key that does not match the seal. An edit refuses, writing nothing, an anchor
   without a salt (zwallet 0.1.0) and a derived key that is not the sealed one ("the
   password-derived whitelist key is not the one wallet.json seals; ..."), which only a policy
   sealed with the password, or a bug, can cause.
   - If it doesn't, and `whitelist.next.json` does, that is the base: a committed edit that died
     before it wrote `whitelist.json`. Its verified bytes are written as `whitelist.json` (from
     memory, never re-read from its path) before the change is staged over
     `whitelist.next.json`.
   - If neither verifies, **an edit refuses** and writes nothing: rebuilding from an empty list
     would drop every entry and loosen every stricter fee cap on the strength of whoever damaged
     the file.
   - Without an anchor (`wallet.json` seals no whitelist) an edit builds on an empty list only
     when there is no list file at all (the first edit). If `whitelist.json` or
     `whitelist.next.json` exists, **an edit refuses** too: a `wallet.json` restored from before
     the first edit (the real list is still there, and a new key at gen 1 would drop it), a
     planted file (whose values would make a loosening look like a tightening), or a first edit
     interrupted between staging and re-seal. The refusal and `list --verify` name the files
     found. A genuinely interrupted first edit leaves only `whitelist.next.json`, at gen 1,
     signed by the key that edit derived under a salt never sealed, and no `whitelist.json`;
     the summary of any edit or reset shows its generation and entries (no command checks its
     signature without a seal).
     A `whitelist.json`, or a staged list above gen 1, means a rollback or a planted file.
   - Both refusals give the reason (the cause, for a `whitelist.json` above the seal by its
     generation and signer: see **Joint rollback**), point to `zwallet whitelist list
     --verify`, and give one recovery whatever the cause: "If wallet.json is an older copy,
     restore the current one if you have it (zwallet keeps no copy of it); otherwise ...". They
     never suggest restoring an older `whitelist.json` (see **zwallet never suggests restoring
     an older whitelist.json**). zwallet keeps no copy of `wallet.json` (`zwallet backup` prints
     only the recovery phrase), so that needs a copy the human made after the last edit.
     `zwallet whitelist reset` starts over, and the message says what it costs: every fee cap
     back to the built-in default, no payee and no contract (native self-transfers under the
     default caps and approve-0 revokes still sign), so `fee-cap` is set again before
     `--unlock`. Reset is for a damaged list, any list that does not verify while the current
     `wallet.json` is in place, or a whitelist made by zwallet 0.1.0 (and, at no anchor, for an
     interrupted first edit, where it loses nothing).
4. Apply the change at `gen' = gen + 1`, validate, sign, and write `whitelist.next.json`. Re-seal
   `wallet.json` with the new anchor (refused if its ciphertext changed since step 2). Then write
   `whitelist.json` from the signed bytes in memory (never re-read from the staged path): the
   change has landed. Then `whitelist.next.json` is removed, best-effort: if the removal or the
   directory sync after it fails, the change still succeeds with a `warning` ("the change
   landed, but zwallet could not remove whitelist.next.json, or could not sync its directory
   after removing it (...). If whitelist.next.json is still there it is not needed
   (whitelist.json holds the list just signed): remove it; a later change can stage its list
   only where there is a plain file or nothing"), and `--unlock` still runs. A
   whitelist.next.json left that way is shown by the next summary as "a copy of whitelist.json
   (e.g. left after a change)". A crash at any point is recovered by step 3 (and by the daemon's
   `next` fallback): either the old generation stands, or the staged one is promoted. A reset
   cut off before its re-seal leaves the old salt sealed, so the next edit derives the old key
   and goes on from the old list. There is one exception, and it loosens nothing: a first edit
   cut off between staging and re-seal: `wallet.json` still seals no whitelist, so the staged
   `whitelist.next.json` makes every edit refuse, and `reset` is needed (it loses nothing, since
   no list was ever sealed).
5. The wallet **stays locked**, unless `--unlock <duration>` was given (the summary says which):
   then a new session starts for that duration, the one typed in this command, with the password
   just typed, through the staged unlock. No duration or expiry is taken from the session that
   was running. The output shows the new `expiresAt` and the daemon's `whitelist` field. Then
   the password is wiped. If the edit fails after step 1, nothing is unlocked and the error says
   that the daemons were already stopped, so the wallet is locked.

The first `add` makes the key: it draws the salt, derives the key, and seals the anchor with the
salt at generation 1. `whitelist reset` draws a new salt, so a new key, and makes an empty list
with the default policy at the next generation (the sealed one + 1, or 1 with no seal): every fee
cap back to the built-in default, no payee and no contract. It is for when the list does not
verify and the current `wallet.json` is in place, or for a whitelist made by zwallet 0.1.0: the
entries are then added again by hand. When the list
it replaces did not verify (or `wallet.json` sealed none), its output shows it as `discarded`
(the reason and its unverified entries as JSON; `null` when the file is missing, cannot be
read or has no entries, and the human output then says on the line of what was dropped "no
verified entry; whitelist.json's entries unknown (it is missing, cannot be read, or has no
entries)"). A `whitelist.next.json` the reset drops that was not the sealed list (nor
`whitelist.json`'s own bytes) is shown as `discardedNext: { entries }` the same way. An edit
never has `discarded`: it refuses instead. A change's output: `{ change, gen, previousGen, base:
"current" | "promoted-next" | "empty", entries: [nicknames], verified: true,
stoppedDaemons, entry? | feeCap? | allowMessageSigning? | dropped?, discarded?, discardedNext?,
warning?, locked, hint? }`, and with `--unlock` `{ duration, expiresAt, daemonPid, whitelist }`
(or `unlockError` if the new session could not start).

**CLI.**
- `whitelist list` works while locked and prints `gen`, the entries and the policy of
  `whitelist.json` as it is, with `verified: false`. Human output shows a value from the file as
  is only when it is short printable ASCII, else quoted with escapes and cut after 64
  characters; messages that echo file values are escaped by the core the same way. With
  `--verify` (password) it checks the list locally and prints `verified: true` with `source`
  (`whitelist.json`, or `whitelist.next.json` after an interrupted edit) and `sealedGen`, or
  `verified: false` with the `reason`. With no seal but a list file, the reason is the one an
  edit refuses with (a rolled-back `wallet.json`, a planted file, or an interrupted first edit,
  naming `whitelist.next.json` when it is there) and the same options, not the first `add`
  (which would refuse). Without `--verify`, a lone `whitelist.next.json` is named too, with a
  pointer to `--verify`.
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
   wallet.json's KDF parameters (replaced on 2026-10-21 by the derived key, below).
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
9. Review round 2: the summary always says what happens to signing; the files are read once
   for the summary and the edit is bound to those bytes (refused if they changed); an edit never
   rebuilds an unverifiable list from empty (the plan's "start from empty and show it as
   discarded" let whoever damaged the file drop every entry and loosen the fee caps), only
   `reset` starts over.
10. Review round 3: a staged list is promoted from its verified bytes, not renamed; an edit also
    refuses list files without a sealed whitelist (rollback to before the first edit, a planted
    file, an interrupted first edit); refusals and the reset summary say what reset costs and
    offer restoring a copy of the list first (removed in round 8); the files are checked again
    before the daemons are stopped; the summary states every outcome and both files' old values,
    cuts name lists at 20, and repeats the change on its last line.
11. Review round 4: with a verified list the daemon decides the action before the fee, so a
    forbidden action with a huge fee is `WHITELIST_DENIED`, never a `fee-cap` command that allows
    nothing; a `whitelist.json` newer than the seal names the `wallet.json` rollback (scoped by
    generation and signer in round 5), and a copy of the list is offered only at the sealed
    generation (removed in round 8); after the re-seal the new list is written from the signed bytes
    in memory and the staged file removed; `list --verify` with no seal but a list file explains why
    every edit refuses instead of offering the first `add`; the reset summary names every generation
    the reset can make, never shows an existing file that cannot be read as "none", and names a
    chain with a cap but no built-in default.
12. Review round 5: a `whitelist.json` above the seal is diagnosed by its generation and signer (two
    or more edits under the sealed key: a rolled-back `wallet.json`; one edit: that or an edit that
    never finished sealing, with the question that tells them apart; another key: a rollback across
    a reset or a foreign file); every other copy remedy holds only for the latest generation the
    human remembers (round 6: the highest; removed in round 8); an edit judges the list before
    `whitelist.key`; removing `whitelist.next.json` after the publish is best-effort, with a
    `warning`; plain `list` names a lone `whitelist.next.json`.
13. Review round 6 (its gate superseded in round 8): the one-edit and other-key reasons use the
    highest generation the human remembers, not "did the last edit report gen N?" (which, after
    edits to M+1 and M+2, sent a human who saw M+2 to the gen-M copy and completed the rollback);
    every copy remedy says "highest"; the removal warning holds whether the removal or the sync
    failed; a staged copy left after a change is labelled as such; `--unlock` follows a change that
    landed with a warning; a reset names a dropped `whitelist.next.json` (`discardedNext`).
14. Review round 7 (text only; the gate superseded in round 8): the gate counts generations since
    the last start at gen 1 (a reset or first edit made with no whitelist sealed starts again
    there), seen in an edit's report or `unlock`, `status`, `list --verify`, not plain `list`; the
    other-key reason also names a reset that never finished sealing; the removal warning tells the
    human to remove a leftover `whitelist.next.json`.
15. Review round 8: the remedy that suggested restoring a copy of `whitelist.json` is deleted,
    with every condition on it (rounds 4 to 7). Each version of the condition had a history in
    which it held and the copy completed a rollback, because every one rests on what the human
    remembers and zwallet keeps no counter outside the home. The reasons keep naming the cause;
    the only recovery is the current `wallet.json` if that is what went back, otherwise
    `reset`. A test scans the code and these docs for any text that suggests restoring a copy.

The derived key (2026-10-21, after v0.1.0; [`v0-1/ssot.md`](v0-1/ssot.md) item 20, the user's
request):
1. The random key in `whitelist.key` is gone with all its code: each edit derives the key from the
   typed password and `whitelistSalt`, sealed next to the anchor (see **The key**). The first edit
   and every reset draw a new salt; an edit checks the derived public key against the sealed one.
2. The daemon's code is unchanged, but what it accepts is not: it keeps `{pub, gen, digest}`
   and drops the salt, after the parse has checked that the salt is there and well-formed. An
   anchor without one is untrusted, so the daemon signs nothing under it (item 3). A test pins
   that its anchor holds nothing else.
3. An anchor sealed by zwallet 0.1.0 (no salt) signs nothing and every edit refuses it until
   `reset`. The daemon's reason is the cause alone; the editor and `list --verify` add the
   recovery.
4. The output's `keyCreated` is dropped rather than renamed: every edit derives the key, and the
   first edit (`previousGen: null`) and a reset (`change: "reset"`) already say when it is new.
5. The crash exception "a reset cut off after writing its new key file" is gone: there is no key
   file to get out of step with the seal.
6. Review: `wallet.json`'s envelope salt must be exactly 32 bytes (it was 16 or more), so the AES
   key can never be scrypt over the whitelist key's domain-prefixed salt. Every zwallet build
   has drawn 32 bytes, so no wallet it wrote is refused.

## Acceptance

| ID | Run | Observe |
|---|---|---|
| W1 | first `whitelist add alice 0x… --chains base` (password) | the change (address, chains, gen 1) is shown before the prompt; a salt sealed in `wallet.json` and no key file written; `list` (locked) shows alice, gen 1, `verified:false`; `--verify` gives `verified:true` |
| W2 | unlocked, no entry for X: `transfer --to X` | `WHITELIST_DENIED` with the exact add command; nothing signed; journal unchanged |
| W3 | `transfer --chain base --to @alice …` | signed and sent; `transfer --chain ethereum --to @alice` gives `WHITELIST_DENIED` (chain scope) |
| W4 | edit `whitelist.json` by hand, adding an address (or delete it) | every value send is `WHITELIST_INVALID`; every edit refuses, writes nothing and leaves a running session alone, pointing to `list --verify` and to the one recovery (the current `wallet.json` if an older one went back, otherwise `reset` with its cost), never to a copy of the list; `reset` starts over and shows what it dropped as `discarded` |
| W5 | swap `whitelist.json` alone for an older signed copy; or restore an older `wallet.json` alone (two edits back, one edit back, from before a reset) | `WHITELIST_INVALID` (gen mismatch); for the older `wallet.json`, the reason, `list --verify` and every edit name, by generation and signer, a rolled-back `wallet.json`, an edit that never finished sealing or a rolled-back `wallet.json`, or a rollback across a reset, a reset that never finished sealing or a foreign file; the one recovery is the current `wallet.json` or `reset`, and no copy of the list is suggested |
| W5b | restore an older `wallet.json` and `whitelist.json` together (the older `wallet.json` seals its own salt) | accepted, as documented. `unlock` prints the older gen and entries, so the human can see it |
| W6 | `approve(spender, 1)`; `transferFrom`; `invoke` of an unlisted contract or an unlisted selector; a token transfer to a token-contract address | each `WHITELIST_DENIED`; `approve(spender, 0)` is allowed |
| W7 | test-hook crash after each of the edit's writes | the list is either the old gen or the promoted new gen; never invalid-and-signed; the next CLI edit continues from it (crashes also after the files are read and after a promotion). A first edit cut off between staging and re-seal leaves only `whitelist.next.json`: every edit refuses, `list` and `list --verify` name it, and `reset` starts over, losing nothing. A reset cut off before its re-seal: the old list and its key stay, and the next edit goes on from them. A `whitelist.next.json` that cannot be removed after the publish: the change succeeds with a `warning`, `--unlock` still starts the session, and the next summary labels the file "a copy of whitelist.json (e.g. left after a change)" |
| W8 | edit while unlocked (tested). A signature racing the edit is not run as a test: it relies on step 1 returning only once no daemon is left | a wrong password or a refused change stops nothing; otherwise every daemon is stopped first (other homes' too) and the wallet stays locked; with `--unlock 10m` a new session runs until now + 10 min and reports the new generation |
| W9 | fees above `maxFeeWei` (hostile RPC fixture, or a large `--max-fee-gwei`) | `FEE_CAP_EXCEEDED` from the daemon for an action the list allows; a forbidden or unlisted action over the cap is `WHITELIST_DENIED`, with no `fee-cap` command |
| W10 | `sign-message` with `allowMessageSigning` off, then on | refused, then signed |
| W11 | 0-value self-transfer dry-run and live (A7), no list | allowed (under the built-in default fee cap); with a list that cannot be trusted, refused (`WHITELIST_INVALID`) |
| W12 | invariant tests | type-4, typed-data and raw-hash requests have no protocol path and are refused |
| W13 | `transfer --dry-run` to an address the list refuses | `whitelist: {allowed: false, …}` with the add command, and no signature request reaches the daemon; an allowed dry-run is signed and shows `allowed: true`; without a `whitelist.json` the daemon is asked and the output has no `whitelist` field |
| W14 | rewrite `whitelist.json` after the summary is shown, before the password is typed; or show a forged list and restore the genuine one | refused, "changed after it was shown" or the list shown does not verify; nothing written, a running session left alone |
| W15 | a change with and without `--unlock` | the summary ends with "a new signing session for <dur> (until …)" or "the wallet stays locked", before the prompt |
| W16 | restore a `wallet.json` from before the first edit under a real list | every edit refuses, nothing written; `reset` starts over; the daemon signs only native self-transfers under the built-in default caps |
| W17 | an edit from an unfinished edit in `whitelist.next.json` | the summary shows both files and every outcome; the change lands on the staged list; `whitelist.json` is written from the verified bytes |
| W18 | edits with the password; a sealed salt that does not derive the sealed key, or another password over the same seal; a seal from zwallet 0.1.0 (no salt) | every edit derives the same key from the same password and salt, and no key file is ever written; a reset draws a new salt and key; a key that is not the sealed one refuses, nothing written; a 0.1.0 seal: `unlock` shows it untrusted, with the cause alone as the reason, and nothing is signed; every edit and `list --verify` add the recovery, every edit refuses before anything is stopped, and `reset` starts over; the daemon's session holds the anchor and never the salt; the key is derived with the KDF parameters that opened `wallet.json`, not ones edited on disk after |

## Not in v0.1 (stays planned)

- Amount caps (a rolling 24 h native cap, per-token caps).
- The `invoke` duplicate key on calldata, and the duplicate window as a policy setting (R4 N-3 and
  N-4).
- User-added tokens.
- Moving the whitelist key to the Secure Enclave or hardware.
