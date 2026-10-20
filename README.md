# zwallet

Local, login-free wallet CLI for agents (EVM today; Solana and TON planned). A Rust
re-implementation of the wallet surface of Trust Wallet's `twak` CLI with everything that needed
Trust's servers removed: no API credentials, no telemetry, no OS keychain, no backend registration.
Keys stay on this machine; chain access is plain JSON-RPC.

It does four things: create a wallet, read balances, call contract functions (read and write), and
transfer. A human unlocks it for a chosen time; while unlocked, a small daemon holds only the key so
an agent can sign without the human present.

## Quick start

```sh
zwallet create main          # human, at a terminal: choose a password (asked twice)
zwallet unlock 24h           # human: unlock signing for 24h (also 30m, 365d, 1y, ...)
zwallet address --verify     # human: 0x..., checked against the seed with the password
zwallet address              # anyone: 0x... from the cache (works while locked)
zwallet balance --chain base
zwallet transfer --chain base --to 0x... --amount 0.01 --dry-run
zwallet transfer --chain base --to 0x... --amount 0.01 --wait
zwallet lock                 # human: wipe the key now (ends every zwallet session of this user)
```

## Commands

Every command takes `--json`. On failure with `--json` the CLI prints
`{ "error": "<msg>", "errorCode": "<CODE>" }` on stdout (plus `txHash` or `refusedTxHash`,
`nonce`, `from`, `journal` after a send, see **Error codes**) and exits 1; without `--json` the
message goes to stderr as `error: <msg> [<CODE>]`, followed by one `txHash: …` /
`refusedTxHash: …` / `nonce: …` / `from: …` / `journal: …` line for each of those fields, exit 1.
Commands that act on a key take `--key <name>` (default: the active key).

| Command | Output (`--json`) | While locked |
|---|---|---|
| `chains` | `[{ key, name, family, symbol, chainId, rpc }]` | works |
| `create <name> [--password-stdin] [--unlock <dur> [--allow-ptrace-scope-0]] [--words 12\|15\|18\|21\|24]` | `{ name, index, address, active, master, home, chains, verified }` (+ `locked, expiresAt, daemonPid` with `--unlock`) | see below |
| `list [--verify [--password-stdin]]` | `{ active, keys: [{ name, index, address, active }], verified, warning? }` | works |
| `change <name>` | `{ active, address, verified }` | works |
| `address [--chain <key>] [--verify [--password-stdin]]` | `{ chain, key, address, verified }` (EIP-55; `--verify` adds `index` and `warning?`) | works |
| `addresses` | `{ key, addresses: [{ chainId: <key>, address }], verified }` | works |
| `status` | `{ wallet, locked, home, daemonPids?, otherDaemonPids, daemonPid?, expiresAt?, active?, address?, verified? }` | works |
| `balance --chain <key> [--token <erc20>] [--rpc <url>]` | `{ chain, key, address, symbol, decimals, balance, raw, verified: false, token? }` | works |
| `call --chain <key> --to <contract> --sig "<fn>" [ARGS...] [--rpc <url>]` | `{ chain, from, to, sig, args, result, raw }` | works |
| `invoke --chain <key> --to <contract> (--sig "<fn>" [ARGS...] \| --data 0x...) [--value <dec>] [--dry-run] [--wait [--wait-timeout <dur>]] [--nonce <n> [--bump <factor>] [--replace <txHash>] \| [--allow-pending] [--allow-duplicate]] [--max-fee-gwei <gwei>] [--rpc <url>]` | transfer's send fields + `{ sig, args, data, value, valueRaw }` | `WALLET_LOCKED` |
| `transfer --chain <key> --to <0x> --amount <dec> [--token <erc20>] [--force] [--dry-run] [--wait [--wait-timeout <dur>]] [--nonce <n> [--bump <factor>] [--replace <txHash>] \| [--allow-pending] [--allow-duplicate]] [--max-fee-gwei <gwei>] [--rpc <url>]` | see below | `WALLET_LOCKED` |
| `sign-message [--chain <key>] --message <s>` | `{ chain, key, address, message, signature, digest }` (EIP-191) | `WALLET_LOCKED` |
| `tx --chain <key> <txHash> [--rpc <url>]` | `{ chain, txHash, found, status, from, to, nonce, value, valueRaw, symbol, explorerUrl, blockNumber?, gasUsed?, success?, hint? }` | works (no wallet needed) |
| `unlock <dur> [--password-stdin] [--allow-ptrace-scope-0]` | `{ locked: false, duration, expiresAt, active, index, address, daemonPid, verified: true, warning? }` | - |
| `lock` | `{ locked: true, wasUnlocked, stopped: [<pid>] }` | - |
| `backup` | prints the recovery phrase; interactive terminal only | asks for the password |

- **Durations** are positional: an integer and one unit of `s m h d w y` (`1y` = 365d). `0`,
  negatives, fractions and unknown units are `VALIDATION_ERROR`.
- **`create <name>`**: on first run it creates the master seed (BIP39, 24 words by default) and key
  `<name>` at index 0, under a new password of at least 12 characters. On a terminal it then
  reminds the human (on stderr) to run `zwallet backup` before funding the wallet. The home's
  parent directory must already exist. Afterwards it derives the next index as `<name>` and makes
  it active. At a terminal or with `--password-stdin` it asks for the password for that one
  operation and derives the key itself (`verified: true`). Without either (an agent) it derives
  through the unlocked daemon without a password (`verified: false`), and while locked it refuses.
  It starts no daemon unless `--unlock` is given. Concurrent creates get distinct indices.
- **Address trust** (`verified`): the addresses in `wallet.json` are a plain cache that anything
  running as the same user can edit, and that user can also swap `wallet.json` for its own wallet
  and run a genuine `zwallet daemon` over it, or put a listener on `d.sock`. So `verified: true`
  only for an address derived from the seed after the human's password, by this process or by
  the daemon child it just started:
  - `address --verify` and `list --verify` (password at the terminal prompt, or
    `--password-stdin`): decrypt `wallet.json` here, derive, and compare with the cache. A
    mismatch is `VALIDATION_ERROR`; a wallet the password does not open is `WRONG_PASSWORD`.
    Both show each key's derivation `index`, and add a `warning` when an index is one zwallet
    never assigns (at or past `nextIndex`, or with an unused index below it): the key list in
    `wallet.json` was edited. Such an address is still this seed's, but a wallet restored from
    the phrase lists accounts in index order and may not show it.
  - `unlock` and `create --unlock`: the address the CLI's own daemon child derived from the seed
    the password opened, reported on that child's channel (both also add `daemonPid`). `unlock`
    also shows that key's derivation `index`, and the same `warning` as `--verify` when the index
    is one zwallet never assigns (checked against the key list it read before the prompt).
  - `create`: the first key, derived from the seed generated in the same process, and a later
    key when the password was asked for (see above).

  Everything else is `verified: false`: `address` and `list` without `--verify`, `addresses`,
  `status`, `change`, an agent's `create` (derived by whatever daemon answers), and `balance` (the
  cache). All but `balance` add the line ``not verified: run `zwallet address --verify` before
  funding`` in human output. While a daemon answers, `address`, `addresses`, `list`, `status` and
  `change` still compare the cache with its derivation and fail with `VALIDATION_ERROR` on a
  mismatch, but a match only means nothing visibly tampered: it is never `verified`. Take a
  funding address from `address --verify` (or `unlock`) on your own terminal.
- **`call` / `invoke` signatures** are `name(argTypes)(returnTypes)`, e.g.
  `balanceOf(address)(uint256)` or `getReserves()(uint112,uint112,uint32)`. Return types are
  optional (without them `result` is null and `raw` has the hex). Arrays are written `[1,2]`,
  tuples `(a,b)`; scalar results are strings. `uint`/`int` mean `uint256`/`int256` (the selector
  is computed from the canonical names). Arguments:
  - `address`: `0x` plus 40 hex digits; mixed case must be a valid EIP-55 checksum.
  - integers: plain decimal (`-` allowed for `int`) or `0x` hex; no units or exponents.
  - `bytes`, `bytesN`, `function`: `0x` hex, exactly N bytes for `bytesN` (24 for `function`).
  - `bool`: `true` or `false`.
  - a top-level `string` argument is taken exactly as given (no trimming, quotes included).
  - inside a list, every element is trimmed. A string element that starts with `"` or `'` must
    end with the same quote, which is removed; quoting keeps commas, brackets and surrounding
    spaces. There are no escapes: to include one kind of quote, quote with the other
    (`["a, b", 'say "hi"', plain words]`).
- **`transfer` / `invoke`** fill the pending nonce, EIP-1559 fees (legacy `gasPrice` where the chain
  has no base fee), `eth_estimateGas` + 20% (exactly 21000 for plain transfers) and, on Base and OP
  Mainnet, the L1 data fee. Without `--dry-run` a shortfall is `INSUFFICIENT_FUNDS` before anything
  is signed. `--dry-run` signs but does not broadcast and returns `{ dryRun: true, chain, key, from,
  to, amount, amountRaw, symbol, decimals, token?, chainId, nonce, pendingNonces, pendingRange,
  pendingCount, duplicateSends?, gasLimit, gasEstimate, gasLimitSource, type, maxFeePerGas,
  maxPriorityFeePerGas, gasPrice, maxGasCost, l1Fee, nativeBalance, sufficientFunds, shortfall,
  signerDecoded, txHash, recoveredFrom }`. It has no `rawTx`: a dry-run never hands out bytes that
  could be broadcast without zwallet. `recoveredFrom` is recovered from the signed raw bytes,
  `signerDecoded` is what the key daemon decoded from the bytes it signed, and an empty wallet can
  be dry-run. `pendingNonces` lists the key's nonces that are pending (between its mined count and
  its next free nonce) on the node, at most the first 64; `pendingRange` is `[first, last]` of them
  (or `null`) and `pendingCount` how many there are. Both counts come from the node, so the range is
  never listed in full. `duplicateSends` (a send without `--nonce`) lists the journal's recent sends
  that this one repeats (see below). Live: `{ ..., txHash, explorerUrl, journal, broadcastWarning?,
  rpcTxHash? }` plus `{ status, blockNumber, gasUsed }` with `--wait`.
- **`--wait`** polls for the receipt for `--wait-timeout <dur>` (default `90s`; same units as
  unlock durations). Not mined by then is `status: "pending"`, and a receipt RPC problem is
  `status: "unknown"` with `receiptError`; neither turns a broadcast into an error, and both add
  ``hint: "still pending; to speed up or cancel use --nonce N --replace <txHash>; a run without
  --nonce is a second payment; look it up with `zwallet tx --chain <chain> <txHash>` or
  <explorer>/tx/<txHash>"`` (for `unknown` it starts "receipt unknown, it may still be pending").
- **`tx --chain <key> <txHash>`** looks a transaction up on the chain's node
  (`eth_getTransactionByHash` and `eth_getTransactionReceipt`). It needs no wallet and no key, so
  it works while locked. `status` is `"mined"` (with `blockNumber`, and `gasUsed` and `success`
  from the receipt), `"pending"`, or `"not found"` (`found: false`, and `from`, `to`, `nonce`,
  `value` are `null`). `value` is in the chain's coin (`valueRaw` in wei); for a token transfer
  it is `0` and `to` is the token contract. A pending transaction adds `hint: "still pending; to
  speed up or cancel use --nonce N --replace <txHash>; a run without --nonce is a second
  payment"`. Not found adds a `hint` that this does not mean it was never sent (it may still
  reach the network, be held by other nodes, or have been dropped) and that a retry of the
  payment passes `--nonce <the nonce it was signed with>`, never a plain resend. A node that
  answers about another transaction is `RPC_ERROR`, never "not found". Every message below that
  says to look a transaction up names this command with the hash and the explorer page.
- **The send journal.** Right before `eth_sendRawTransaction`, zwallet appends one JSON line
  `{ ts, chain, from, nonce, txHash, to, value, amountRaw, kind, token? }` (`value` like
  `"0.01 ETH"` for people, `amountRaw` the same amount in base units: wei, or the token's smallest
  unit; `kind` `transfer`, `token-transfer` or `invoke`, `to` the recipient or the contract,
  `token` the token contract of a token transfer) to
  `sends-<chainId>-<address>.jsonl` in the wallet home (0600, fsynced), and prints the same fields
  on one stderr line: "broadcasting <txHash> (nonce N on <chain>); if this command does not finish,
  look this hash up before sending again (`zwallet tx --chain <chain> <txHash>` or
  <explorer>/tx/<txHash>); a retry must pass --nonce N --replace <txHash>. Journal <path>: {…}".
  So a run that is killed after the broadcast still leaves its hash behind. If the line cannot be
  written, nothing is broadcast. A record cut off by a killed run (no final newline) gets a
  newline before the next one, so it never swallows that line. The journal is opened without
  following a symlink and without blocking (a FIFO cannot hang a send), and must be a regular
  file owned by this user; otherwise a send without `--nonce` is `VALIDATION_ERROR` "cannot read
  the send journal <path>: …; nothing was signed", since the repeat check below needs it. Errors
  and messages below that send the agent to an earlier transaction name the journal's entries at
  that nonce, each with its `zwallet tx` command and explorer page.
- **One send at a time.** Sends from one key on one chain never overlap: each holds
  `nonce-<chainId>-<address>.lock` in the wallet home from before reading the nonce until the
  command returns, through `--wait`. A send that finds the lock held does not wait (after waiting,
  a second attempt at the same payment would sign the next nonce and land too): it fails at once
  with `VALIDATION_ERROR`, before any RPC call, so nothing is signed or broadcast (see **Error
  codes**). The message says where the running send's txHash will be (its output, and the send
  journal once it is broadcast) and how to look it up. A dry-run takes no lock.
- **A send without `--nonce` while one is pending is refused.** If the key has a transaction pending
  on the node (its next free nonce is above its mined count), a send without `--nonce` is
  `VALIDATION_ERROR` "a transaction from this key is still pending on <chain> at nonce N; nothing
  was signed", with the journal's entries at those nonces (`pendingNonces`, `pendingRange`,
  `pendingCount`, `pendingSends`, `journal`). The usual cause is a re-run of a send that was cut
  off, which would land at the next nonce as a second payment: look the pending transaction up, and
  to speed it up or replace it use `--nonce N --replace <its txHash>`. `--allow-pending` sends
  anyway; use it only for a different payment. A dry-run reports `pendingNonces` instead of
  refusing.
- **A send without `--nonce` that repeats a recent one is refused.** If the send journal has an
  entry from the last 10 minutes with the same chain, kind, `to` and amount in base units
  (`amountRaw`: wei for a native transfer or an invoke's value; for a token transfer the token's
  base units and the same token contract), a send without `--nonce` is `VALIDATION_ERROR` "this
  looks like a repeat of a recent send; look those hashes up first. If this really is a second
  payment, pass --allow-duplicate", naming each match's txHash, nonce and time with its `zwallet
  tx` command (`duplicateSends`, `journal`); nothing is signed. The display text (`value`) is
  never compared: a token's symbol comes from the node, and a `symbol()` call that fails once
  (shown as `?`) must not make a repeat look new. A journal line written before `amountRaw`
  existed is compared by its `value`. This catches the re-run of a send that was cut off after it
  was already mined, which the pending check cannot see. A send that was refused or never got out is
  in the journal too, so the lookup comes first there as well. `--allow-duplicate` sends anyway;
  a dry-run reports `duplicateSends` instead of refusing, and a live send without `--nonce`
  reports it too. `--nonce` sends skip the check: at a fixed nonce they can only replace or retry,
  never add a payment. Only the journal's last 1 MiB is read, and lines that are not entries are
  skipped.
- **`signerDecoded.action`** names the ABI method the calldata calls, never an asset, because
  ERC-20 and ERC-721 share these selectors: `{ kind: "native-transfer", to, value }`,
  `{ kind: "transfer-call", token, to, amount, assetType: "unverified" }`,
  `{ kind: "approve-call", contract, spender, value, assetType: "unverified" }`,
  `{ kind: "transfer-from-call", contract, from, to, value, assetType: "unverified" }`, or
  `{ kind: "contract-call", contract, selector, value }`. Amounts are raw base units.
- **`--nonce <n>`** signs with nonce `n` instead of the next free one; `n` must be between the
  account's mined count and its next free nonce (`VALIDATION_ERROR` otherwise). A mined `n` is "the
  transaction at nonce N is already mined; nothing was signed. If it was your earlier attempt at
  this payment, the payment went through; look it up before sending anything new", with the
  journal's entries at `n`. `n` equal to the next free nonce takes that free slot. Below it,
  `--nonce` replaces the transaction pending at `n`, and `--replace <txHash>` must name it (it must
  be this key's, at nonce `n`, not mined); without it the send is `VALIDATION_ERROR` "nonce N
  belongs to a pending transaction; pass --replace <its txHash> to replace it (see the send
  journal)", with the journal's entries at `n`. That message, the mined-nonce one and the
  unknown-`--replace` one below end with "If the transaction at nonce N is a different payment than
  the one you are retrying, do not replace it; that payment and yours are separate". A `--replace`
  hash that is already mined is refused: if it was your earlier attempt at this payment, the payment
  went through; look it up. zwallet reads the replaced transaction's fees from the node and pays
  `--bump` times the higher of them and its own estimate (tip and max fee, or `gasPrice`; default
  1.25, allowed 1.1 to 3) so that nodes accept the replacement. An explicit `--bump` also applies
  when nothing is replaced. If the node does not know the `--replace` hash: where it holds another
  pending transaction at `n`, the error says the hash is the wrong one and names the journal's
  entries at `n`; where nothing is pending at `n`, it says to retry with `--nonce n` and no
  `--replace`. With `--nonce` the output adds `replacesPending` and `feeBump` (e.g. `"1.25x"`, or
  `null` without a bump), and `--replace` adds `replacesTx`.
- **`--max-fee-gwei <gwei>`**: fees come from the RPC node, so zwallet refuses a tip above 2x and a
  price per gas above 6x of max(base fee, `eth_gasPrice`, 1 gwei) with `VALIDATION_ERROR` before
  signing. On a legacy chain (no base fee, e.g. BSC) the price is `eth_gasPrice` itself, so it is
  left out of its own ceiling: `gasPrice` may be at most 6x of max(base fee, 1 gwei), i.e. 6 gwei.
  `--max-fee-gwei` replaces these ceilings with one explicit price per gas (higher or lower); it
  is checked after the bump.
- **`--force`** (`transfer`) allows a recipient that is almost always a mistake: the zero address,
  or the token contract itself. Without it they are `INVALID_ADDRESS`.
- **Amounts** are decimal strings converted exactly (no floats); more fractional digits than the
  asset's decimals is `VALIDATION_ERROR`. ENS names are refused (`INVALID_ADDRESS`).
- **Error codes**: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`, `WRONG_PASSWORD`,
  `WALLET_LOCKED`, `KEY_EXISTS`, `KEY_NOT_FOUND`, `CHAIN_UNSUPPORTED`, `CAPABILITY_UNSUPPORTED`,
  `INVALID_ADDRESS`, `RPC_ERROR`, `INSUFFICIENT_FUNDS`, `BROADCAST_UNCERTAIN`. Errors from the
  broadcast step carry the signed transaction's `txHash` (or `refusedTxHash`, below), `nonce`,
  `from` and, once the journal line is written, `journal`. zwallet never suggests a plain resend:
  a retry of a send that may have gone out passes `--nonce <nonce> --replace <txHash>`, so it can
  only replace the first attempt, never add a second one.
  - A failure after signing but before the send request keeps its code (`RPC_ERROR` when the node
    is unreachable) and says "...; nothing was broadcast. If you retry, pass --nonce N ...":
    nothing went out, so nonce N is still free and there is nothing to name with `--replace`.
  - `BROADCAST_UNCERTAIN`: the send failed or timed out, or the node answered with another hash
    (`rpcTxHash`), and the node does not report the transaction by its own hash. It may still be
    on the network. Look it up first (the message names `zwallet tx --chain <chain> <txHash>`
    and the explorer page), or retry with `--nonce <nonce> --replace <txHash>` so the retry
    replaces it instead of sending twice.
  - "nonce N is taken by another transaction from this key, pending or mined": the node answered
    "nonce too low" or "replacement transaction underpriced" and does not report this transaction
    by its hash. This transaction was refused and will never be found, so its hash is
    `refusedTxHash`, not `txHash`, and the message names the journal's other entries at nonce N.
    Without `--nonce` it is `RPC_ERROR`: "If you were retrying an earlier send, look up that
    send's txHash before sending again". With `--nonce` (a retry) it is `BROADCAST_UNCERTAIN`:
    "This retry was refused and will not be found; look up the txHash of your earlier send at
    nonce N from its own output (or the send journal below). While it may be pending or mined, do
    not send this payment without --nonce; to replace it use --nonce N --replace <earlier
    txHash>" (for "underpriced", with a larger `--bump`).
  - `RPC_ERROR` "the node rejected the transaction; nothing was broadcast" when the node refuses
    it outright ("insufficient funds", "intrinsic gas too low", "exceeds block gas limit", "max
    fee per gas less than block base fee", or "transaction underpriced", a price below the pool's
    minimum); it also says to pass `--nonce N` on a retry, and for "transaction underpriced" to
    pay more with `--nonce N --bump <factor>`.
  - A send error or another hash for a transaction the node does report by its own hash (e.g.
    "already known", or "nonce too low" for this very transaction) is a success with
    `broadcastWarning` (and `rpcTxHash` if the node answered another hash).
  - `VALIDATION_ERROR` "another send from this key on <chain> is in progress (lock <path>)":
    nothing was signed or broadcast. If the running send is an earlier attempt at this same
    payment, wait for it to finish and look up its result before sending again (its txHash is in
    its output and, once broadcast, in the send journal; `zwallet tx --chain <chain> <txHash>`);
    for a different payment, retry after it finishes.
  - `VALIDATION_ERROR` "this looks like a repeat of a recent send" and "a transaction from this
    key is still pending on <chain> at nonce N" (a send without `--nonce`, see above) and "nonce N belongs to a pending transaction; pass
    --replace <its txHash>" (`--nonce` below the next free nonce): nothing was signed.

## Chains

| key | chainId | symbol | default RPC |
|---|---|---|---|
| ethereum | 1 | ETH | https://ethereum-rpc.publicnode.com |
| base | 8453 | ETH | https://base-rpc.publicnode.com |
| arbitrum | 42161 | ETH | https://arbitrum-one-rpc.publicnode.com |
| optimism | 10 | ETH | https://optimism-rpc.publicnode.com |
| polygon | 137 | POL | https://polygon-bor-rpc.publicnode.com |
| bsc | 56 | BNB | https://bsc-rpc.publicnode.com |
| avalanche | 43114 | AVAX | https://avalanche-c-chain-rpc.publicnode.com |
| sepolia | 11155111 | ETH | https://ethereum-sepolia-rpc.publicnode.com |

RPC precedence: `--rpc <url>`, then `ZWALLET_RPC_<KEY>` (e.g. `ZWALLET_RPC_BASE`), then the
default. zwallet checks the endpoint's `eth_chainId` before using it. An RPC URL may carry an API
key, so zwallet prints only its scheme and host (`https://host/<redacted>`), parsing the URL the
way the HTTP client does. Some providers put the key in a subdomain, so a host label that looks
like a key (a hyphen-free run of 20+ characters, or 12+ characters mixing letters and digits) is
`<redacted>` too wherever the URL or its origin is printed (`chains`, HTTP errors, a chainId
mismatch); the default hosts above stay readable. A non-2xx HTTP answer is reported as
`HTTP <status> from <scheme://host:port>`, never with its body, and an answer that is not the
JSON-RPC expected only by the parse error's kind and position, never the value it quotes. In other
error text the endpoint sends back, zwallet replaces with `<redacted>` the URL's userinfo, query
values, whole path and key-like host labels at any length, and its path segments (and fragment)
of 8+ characters, each as written and percent-decoded; a part shorter than 8 characters only where
it stands as a whole word. A JSON-RPC error's `data` is printed only when it is a string (revert
data).

## Custody model

- **Home**: `$ZWALLET_HOME` or `~/.zwallet`, mode 0700.
- **`wallet.json`** (0600, written atomically): a sealed section encrypted with AES-256-GCM under a
  scrypt key (N=2^18, r=8, p=1, 256 MiB; parameters, salt and nonce stored in the file; only r=8,
  p=1 and N from 2^15 to 2^18 are accepted, checked before any key derivation), plus non-secret
  metadata: `createdAt`, `active`, `nextIndex`, and
  `keys: [{ name, index, createdAt, addresses: { <chain>: <address> } }]`. The sealed plaintext is
  a versioned object `{ version, mnemonic, policy }`: one BIP39 English mnemonic (no BIP39
  passphrase) and a `policy` object reserved for a future signing policy (empty for now, carried
  through intact). The policy lives under the password because anything outside it can be edited
  by the same OS user without the password.
- **Password**: held by the human only, at least 12 characters. There is no password file and no
  password env variable; it is typed at a terminal prompt (no echo) or piped with
  `--password-stdin`, and read with raw reads into one wiped buffer. Nothing on disk decrypts the
  wallet by itself. `unlock` hands it to its own daemon child over a socketpair, never over the
  socket file (see **Daemon**). `create` writes a
  temp file, reads it back from disk, decrypts it and checks the addresses and the phrase, and only
  then links it into place as `wallet.json`; a failure before that removes only its temp file (a
  home it created stays, with `w.lock`). Once `wallet.json` is in place it is never removed: a
  later failure says recovery is needed and names the file. Every write of `wallet.json` holds an
  exclusive lock on `w.lock` (an empty file, never deleted). `wallet.json` is only read without
  following a symlink and without blocking, and only as a regular file of this user's of at most
  1 MiB, so a FIFO or link planted in its place fails fast instead of hanging a reader.
- **Keys**: named subkeys of the master by index, in creation order, never reused. EVM uses
  `m/44'/60'/0'/0/<index>` (the MetaMask/Trust account order), so the phrase imports into standard
  wallets with the same accounts. One address per key on every EVM chain. Indices stay below
  2^31, where BIP32's hardened range starts (`VALIDATION_ERROR` at or above it).
- **Daemon**: `unlock` pins the zwallet binary before it asks for the password. On Linux that is
  the running image (`/proc/self/exe`), so a file swapped in at the binary's path while the human
  types is never started. On macOS it is the file's identity, checked again right before the
  start: a binary swapped in while the human types is refused, but one swapped in between that
  check and the exec is not (macOS has no `fexecve`), and one swapped in before `unlock` ran gets
  the password on any OS (see **Known limits**). On Linux, `unlock` and `create --unlock` also
  refuse, before the password prompt, when `kernel.yama.ptrace_scope` is 0 or Yama is missing
  (any same-user process could attach to the new daemon during its start-up); set it to 1 or
  more, or pass `--allow-ptrace-scope-0` to accept that. `unlock` then starts the pinned binary
  as its own child `zwallet daemon --home <canonical home>` (own session, empty environment, no
  inherited descriptor but its channel: the open descriptors are listed and marked
  close-on-exec, not just those below the soft descriptor limit; stderr is `/dev/null`). The
  channel is one end of a socketpair on the child's fds 0 and 1, whose other end only `unlock`
  holds: unlike a pipe, a socket cannot be reopened through `/proc/<pid>/fd`, so no same-user
  process can read the password from it or write a fake readiness line into it, even before the
  child turns non-dumpable (on Linux the daemon refuses to start on anything but a socket). Over
  that channel:
  1. the child, whose first step (like every zwallet process's) disabled core dumps and, on
     Linux, made it non-dumpable, asks for input;
  2. `unlock` writes the TTL and password into the channel. The password never goes over the
     socket file, argv or env;
  3. the child decrypts the wallet and says so. A wrong password ends here, and a session that was
     already running is left untouched;
  4. only now does `unlock` stop the daemons already serving this home (see **`lock`**), then
     tells the child to go on;
  5. the child takes an exclusive lock on `d.lock` for its whole life (waiting up to 5 s),
     replaces whatever is at `$ZWALLET_HOME/d.sock`, binds it (0600; peer uid checked on both
     ends), and reports `{ locked, expiresAt, active, index, address, pid }`.

  `unlock` reports success only after that line, and kills the child on any failure or timeout.
  If `unlock` is gone by then (killed after it told the child to go on), the child's readiness
  write fails, and it wipes the seed and exits instead of serving.
  Concurrent unlocks take turns on `u.lock`, held from after the password prompt until the new
  daemon is ready or has failed (another `unlock` waits up to 30 s for it). A listener squatting
  on `d.sock` gets only requests that carry nothing and is replaced; a process holding `d.lock`
  makes `unlock` fail visibly. Neither ever receives the password.
- **`lock`** is the panic button: it ends every zwallet session of this user, not only this
  home's, so a home moved or re-linked since its unlock cannot hide a daemon. It reads no wallet
  home at all (it works with `ZWALLET_HOME` and `HOME` unset, and nothing at a home path, such as
  a stalled mount, can hold it up): it finds every process of this user whose arguments are
  `daemon --home <absolute path>` in the OS process table (macOS `KERN_PROCARGS2`, Linux
  `/proc/<pid>/cmdline`), sends it SIGTERM (with SIGCONT, so a stopped daemon still wipes its
  seed), and SIGKILL after 2 s. It reports `locked: true` with the pids in `stopped` only when
  none is left; otherwise it fails naming the pids. `unlock` stops the old daemons of its own
  home the same way (the home matched by path first, then by inode, with each stat of another
  process's path bounded to 1 s so a hanging mount cannot stall it).
  `status` reads the same table: any daemon process for this home means `locked: false` (with
  `daemonPids`), whatever answers on `d.sock`; `daemonPid` and `expiresAt` come from `d.sock`'s
  reply only when that reply is from one of those processes. `otherDaemonPids` (always present)
  lists this user's daemons for other homes, which `zwallet lock` stops too; human output says so
  when there are any. On macOS a process list that fills its buffer is read again with more
  room, and one that still fills it is an error, never a shorter list. File names (`d.sock`,
  `d.lock`) never decide which daemons exist, since any same-user process can rename or replace
  them.
- **Daemon duties**: the daemon keeps only the master seed in memory, and does nothing but derive
  addresses and sign; it makes no network request. For a transaction it receives only the
  unsigned signing payload bytes and decodes them itself: type, chain id, recipient, value, and
  the ABI method the calldata calls (`transfer-call` / `approve-call` / `transfer-from-call`, each
  `assetType: "unverified"`, else `contract-call`; see **`signerDecoded.action`**). It refuses
  anything zwallet does not produce (other tx types, access lists, contract creation,
  non-canonical encodings, calldata for those three methods that is not their canonical ABI
  encoding or that also sends native value) and signs exactly the bytes it decoded. It never
  trusts intent fields from the CLI, which is the agent-controlled side. On `lock`, at expiry
  (wall clock), or on SIGTERM it wipes the seed and exits. A reboot or a dead daemon means locked.
  Everything else (RPC, ABI, tx building, broadcast) runs in the one-shot CLI process, which
  checks every signature it gets back by recovering the signer.
- **Daemon lifetime**: each connection is served in its own task with a 10 s I/O budget, so a
  silent client never delays expiry. Every second and on every request (including `status`) the
  daemon also checks that `wallet.json`'s ciphertext is still the one it decrypted and that
  `d.sock` is still its own socket; otherwise it wipes the seed and exits. The socket check is
  best effort (a same-user process can race it); `lock` does not depend on it. Every zwallet
  process disables core dumps as its first step, and on Linux also becomes non-dumpable (out of
  reach of same-user `/proc/<pid>/mem`, `/proc/<pid>/fd` and new ptrace attachments) from that
  step on; not before it, since exec makes every process dumpable again.
- **Secrets are never printed**: the mnemonic, password and private keys appear in no output, log
  or error. The single exception is `backup`, which prints the phrase only when stdin and stdout
  are both an interactive terminal, after the password is entered again.

## Known limits (same user)

The threat zwallet is built against is an agent (or anything prompt-injected into one) running as
the same OS user, with read and write access to the wallet home: it must never get the password or
the phrase, `lock` must end all signing, and no output may present its address as verified. Some
things a same-user process can still do, and what to do about them:

- **Swap the zwallet binary.** A binary replaced before `unlock` (or `create`, `backup`,
  `--verify`) runs gets the password; on macOS so does one swapped in between `unlock`'s identity
  check and its exec of the daemon (there is no `fexecve`). A Homebrew-installed binary is
  writable by the user, and so by the agent. For real protection install zwallet where the agent
  cannot write (e.g. a root-owned directory), or run the agent as a different OS user.
- **Read the human's terminal** while the password is typed: a same-user process can open the
  terminal device the prompt reads.
- **Read a piped password** (`--password-stdin` from a pipe, Linux): the process writing into the
  pipe, and zwallet itself until its first step makes it non-dumpable, are dumpable, so a
  same-user process can open the pipe through `/proc/<pid>/fd` and race zwallet to read the
  password. This is the same class as reading the terminal; prefer the terminal prompt.
- **Log keystrokes under X11**, where any client can read another's input.
- **Attach a debugger on macOS**: a same-user debugger (where the system allows one) can read a
  zwallet process's memory.
- **Linux with `kernel.yama.ptrace_scope` 0** (or without Yama): any same-user process can
  attach to a new daemon before it turns non-dumpable and stay attached (or copy its descriptors
  with `pidfd_getfd`, which takes the same permission). `unlock` refuses this by
  default (`--allow-ptrace-scope-0` accepts it). With `ptrace_scope` 1, a process that is an
  ancestor of `zwallet unlock` (e.g. the shell an agent started, if the human types `unlock`
  into it) still can: run `unlock` from a terminal the agent did not start.
- **Linux hosts with `/proc` mounted `hidepid`** (`1`/`noaccess`, `2`/`invisible`,
  `4`/`ptraceable`, with or without `gid=`) are refused: there the kernel hides a non-dumpable
  process such as the key daemon even from its own user, so `lock` could not find a daemon to
  stop. `unlock` and `create --unlock` refuse to start one before the password prompt (there is
  no flag to accept this), and `lock` and `status` fail instead of reporting the wallet locked.
  The same holds when `/proc/self/mountinfo` cannot be read. `subset=pid` is fine.
- **Root** defeats all of this.

## Development

Gate (must be green before every commit):

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Live tests against public mainnet RPC (read-only; nothing is broadcast):

```sh
cargo test -- --ignored
```

Tests always use a temporary `ZWALLET_HOME` under `/tmp/zwt-*` (short, for the socket path limit)
and never touch the real `~/.zwallet`. Layout: `src/core` (custody, seed, amounts, durations,
errors, daemon protocol), `src/chains` (registry and family traits; `src/chains/evm`),
`src/daemon` (key holder), `src/cli.rs` and `src/commands.rs`. The spec lives in `.prd/`.
