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
zwallet address              # anyone: 0x... (works while locked)
zwallet balance --chain base
zwallet transfer --chain base --to 0x... --amount 0.01 --dry-run
zwallet transfer --chain base --to 0x... --amount 0.01 --wait
zwallet lock                 # human: wipe the key from memory now
```

## Commands

Every command takes `--json`. On failure with `--json` the CLI prints
`{ "error": "<msg>", "errorCode": "<CODE>" }` on stdout (plus `txHash`, `nonce`, `from` after a
send, see **Error codes**) and exits 1; without `--json` the message goes to stderr, exit 1.
Commands that act on a key take `--key <name>` (default: the active key).

| Command | Output (`--json`) | While locked |
|---|---|---|
| `chains` | `[{ key, name, family, symbol, chainId, rpc }]` | works |
| `create <name> [--password-stdin] [--unlock <dur>] [--words 12\|15\|18\|21\|24]` | `{ name, index, address, active, master, home, chains }` (+ `locked, expiresAt, daemonPid` with `--unlock`) | see below |
| `list` | `{ active, keys: [{ name, index, address, active }], verified }` | works |
| `change <name>` | `{ active, address, verified }` | works |
| `address [--chain <key>]` | `{ chain, key, address, verified }` (EIP-55) | works |
| `addresses` | `{ key, addresses: [{ chainId: <key>, address }], verified }` | works |
| `status` | `{ wallet, locked, home, daemonPid?, expiresAt?, active?, address?, verified? }` | works |
| `balance --chain <key> [--token <erc20>] [--rpc <url>]` | `{ chain, key, address, symbol, decimals, balance, raw, token? }` | works |
| `call --chain <key> --to <contract> --sig "<fn>" [ARGS...] [--rpc <url>]` | `{ chain, from, to, sig, args, result, raw }` | works |
| `invoke --chain <key> --to <contract> (--sig "<fn>" [ARGS...] \| --data 0x...) [--value <dec>] [--dry-run] [--wait] [--nonce <n>] [--max-fee-gwei <gwei>] [--rpc <url>]` | transfer's send fields + `{ sig, args, data, value, valueRaw }` | `WALLET_LOCKED` |
| `transfer --chain <key> --to <0x> --amount <dec> [--token <erc20>] [--force] [--dry-run] [--wait] [--nonce <n>] [--max-fee-gwei <gwei>] [--rpc <url>]` | see below | `WALLET_LOCKED` |
| `sign-message [--chain <key>] --message <s>` | `{ chain, key, address, message, signature, digest }` (EIP-191) | `WALLET_LOCKED` |
| `unlock <dur> [--password-stdin]` | `{ locked: false, duration, expiresAt, active, address, daemonPid }` | - |
| `lock` | `{ locked: true, wasUnlocked }` | - |
| `backup` | prints the recovery phrase; interactive terminal only | asks for the password |

- **Durations** are positional: an integer and one unit of `s m h d w y` (`1y` = 365d). `0`,
  negatives, fractions and unknown units are `VALIDATION_ERROR`.
- **`create <name>`**: on first run it creates the master seed (BIP39, 24 words by default) and key
  `<name>` at index 0, under a new password of at least 12 characters. On a terminal it then
  reminds the human (on stderr) to run `zwallet backup` before funding the wallet. Afterwards it
  derives the next index as `<name>` and makes it active. While unlocked this needs no password
  (the daemon derives); while locked it asks for the password for that one operation and starts
  no daemon. Concurrent creates get distinct indices.
- **Address verification**: the addresses in `wallet.json` are a plain cache that anything running
  as the same user can edit. While unlocked, `address`, `addresses`, `list`, `status` and `change`
  check the cache against the key daemon's own derivation from the seed and fail with
  `VALIDATION_ERROR` on a mismatch; their output carries `verified: true`. While locked they print
  the cache with `verified: false` (and a note in human output). `unlock` and `create --unlock`
  print the address the new daemon derived from the decrypted seed, and both add `daemonPid`.
- **`call` / `invoke` signatures** are `name(argTypes)(returnTypes)`, e.g.
  `balanceOf(address)(uint256)` or `getReserves()(uint112,uint112,uint32)`. Return types are
  optional (without them `result` is null and `raw` has the hex). Arrays are written `[1,2]`,
  tuples `(a,b)`; scalar results are strings.
- **`transfer` / `invoke`** fill the pending nonce, EIP-1559 fees (legacy `gasPrice` where the chain
  has no base fee), `eth_estimateGas` + 20% (exactly 21000 for plain transfers) and, on Base and OP
  Mainnet, the L1 data fee. Without `--dry-run` a shortfall is `INSUFFICIENT_FUNDS` before anything
  is signed. `--dry-run` signs but does not broadcast and returns `{ dryRun: true, chain, key, from,
  to, amount, amountRaw, symbol, decimals, token?, chainId, nonce, gasLimit, gasEstimate,
  gasLimitSource, type, maxFeePerGas, maxPriorityFeePerGas, gasPrice, maxGasCost, l1Fee,
  nativeBalance, sufficientFunds, shortfall, signerDecoded, txHash, recoveredFrom }`. It has no
  `rawTx`: a dry-run never hands out bytes that could be broadcast without zwallet.
  `recoveredFrom` is recovered from the signed raw bytes, `signerDecoded` is what the key daemon
  decoded from the bytes it signed, and an empty wallet can be dry-run. Live: `{ ..., txHash,
  explorerUrl, broadcastWarning? }` plus `{ status, blockNumber, gasUsed }` with `--wait`
  (`status: "pending"` if not mined within 5 minutes; a receipt problem never turns a broadcast
  into an error).
- **`--nonce <n>`** signs with nonce `n` instead of the next free one; `n` must be between the
  account's mined count and its next free nonce (`VALIDATION_ERROR` otherwise). Below the next free
  nonce it replaces the pending transaction: zwallet then pays at least 1.25x its own fee estimate
  (tip and max fee, or `gasPrice`) so that nodes accept the replacement. With `--nonce` the output
  adds `replacesPending` and `feeBump` (`"1.25x"` when replacing, else `null`).
- **`--max-fee-gwei <gwei>`**: fees come from the RPC node, so zwallet refuses a tip above 2x and a
  price per gas above 6x of max(base fee, `eth_gasPrice`, 1 gwei) with `VALIDATION_ERROR` before
  signing. `--max-fee-gwei` replaces that ceiling with one explicit price per gas (higher or lower);
  it is checked after the replacement bump.
- **`--force`** (`transfer`) allows a recipient that is almost always a mistake: the zero address,
  or the token contract itself. Without it they are `INVALID_ADDRESS`.
- **Amounts** are decimal strings converted exactly (no floats); more fractional digits than the
  asset's decimals is `VALIDATION_ERROR`. ENS names are refused (`INVALID_ADDRESS`).
- **Error codes**: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`, `WRONG_PASSWORD`,
  `WALLET_LOCKED`, `KEY_EXISTS`, `KEY_NOT_FOUND`, `CHAIN_UNSUPPORTED`, `CAPABILITY_UNSUPPORTED`,
  `INVALID_ADDRESS`, `RPC_ERROR`, `INSUFFICIENT_FUNDS`, `BROADCAST_UNCERTAIN`. Once a signed
  transaction is handed to the node:
  - `BROADCAST_UNCERTAIN`: the send failed or timed out and the node does not report the
    transaction by its hash. It may still be on the network; the error carries `txHash`, `nonce`
    and `from`. Look it up, or retry with `--nonce <nonce>` so the retry replaces it instead of
    sending twice.
  - `RPC_ERROR` "the node rejected the transaction; nothing was broadcast", with the same three
    fields, when the node refuses it outright ("replacement transaction underpriced", "nonce too
    low", "insufficient funds", "intrinsic gas too low", "exceeds block gas limit", "max fee per gas
    less than block base fee").
  - `RPC_ERROR` with `rpcTxHash` (and the three fields) if the node answers with a different hash.
  - A send error for a transaction the node does report by hash (e.g. "already known") is a
    success with `broadcastWarning`.

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
key, so zwallet prints only its scheme and host (`https://host/<redacted>`), and replaces the
URL's path, query string and their parts of 8+ characters with `<redacted>` in any error text the
endpoint sends back.

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
  wallet by itself. `unlock` hands it to its own daemon child over a pipe, never over the socket
  (see **Daemon**). `create` writes a
  temp file, reads it back from disk, decrypts it and checks the addresses and the phrase, and only
  then links it into place as `wallet.json`; a failure before that removes only what it created.
  Once `wallet.json` is in place it is never removed: a later failure says recovery is needed and
  names the file. Every write of `wallet.json` holds an exclusive lock on `w.lock` (an empty file).
- **Keys**: named subkeys of the master by index, in creation order, never reused. EVM uses
  `m/44'/60'/0'/0/<index>` (the MetaMask/Trust account order), so the phrase imports into standard
  wallets with the same accounts. One address per key on every EVM chain.
- **Daemon**: `unlock` first asks any daemon already serving to lock, then starts `zwallet daemon`
  as its own child (own session) and writes the TTL and password into the child's stdin pipe; the
  password never goes over the socket, argv or env. The child takes an exclusive lock on `d.lock`
  for its whole life (one daemon per home), decrypts the wallet, replaces whatever is at
  `$ZWALLET_HOME/d.sock` and binds it (0600; peer uid checked on both ends), and only then reports
  `{ expiresAt, active, address }` back on its stdout pipe; `unlock` reports success only after that
  line, and kills the child on any failure or timeout. Something else squatting on `d.sock` or
  `d.lock` makes `unlock` fail visibly; it never receives the password. The daemon keeps only the
  master seed in memory, and does nothing but derive addresses and sign; it
  makes no network request. For a transaction it receives only the unsigned signing payload bytes,
  decodes them itself (type, chain id, recipient, value, and ERC-20 `transfer` / `approve` /
  `transferFrom` from the calldata), refuses anything zwallet does not produce (other tx types,
  access lists, contract creation, non-canonical encodings), and signs exactly the bytes it decoded.
  It never trusts intent fields from the CLI, which is the agent-controlled side. On `lock`, at expiry (wall clock), or on SIGTERM it wipes the seed and
  exits. A reboot or a dead daemon means locked. Everything else (RPC, ABI, tx building, broadcast)
  runs in the one-shot CLI process, which checks every signature it gets back by recovering the
  signer.
- **Daemon lifetime**: each connection is served in its own task with a 10 s I/O budget, so a
  silent client never delays expiry. Every second and on every request (including `status`) the
  daemon also checks that `wallet.json`'s ciphertext is still the one it decrypted and that
  `d.sock` is still its own socket; otherwise it wipes the seed and exits. Core dumps are disabled
  in the daemon and in `create`, `unlock` and `backup`.
- **Secrets are never printed**: the mnemonic, password and private keys appear in no output, log
  or error. The single exception is `backup`, which prints the phrase only when stdin and stdout
  are both an interactive terminal, after the password is entered again.

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
