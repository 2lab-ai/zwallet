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
`{ "error": "<msg>", "errorCode": "<CODE>" }` on stdout and exits 1; without `--json` the message
goes to stderr, exit 1. Commands that act on a key take `--key <name>` (default: the active key).

| Command | Output (`--json`) | While locked |
|---|---|---|
| `chains` | `[{ key, name, family, symbol, chainId, rpc }]` | works |
| `create <name> [--password-stdin] [--unlock <dur>] [--words 12\|15\|18\|21\|24]` | `{ name, index, address, active, master, home, chains }` | see below |
| `list` | `{ active, keys: [{ name, index, address, active }] }` | works |
| `change <name>` | `{ active, address }` | works |
| `address [--chain <key>]` | `{ chain, key, address }` (EIP-55) | works |
| `addresses` | `{ key, addresses: [{ chainId: <key>, address }] }` | works |
| `status` | `{ wallet, locked, home, expiresAt?, active?, address?, daemonPid? }` | works |
| `balance --chain <key> [--token <erc20>] [--rpc <url>]` | `{ chain, key, address, symbol, decimals, balance, raw, token? }` | works |
| `call --chain <key> --to <contract> --sig "<fn>" [ARGS...] [--rpc <url>]` | `{ chain, from, to, sig, args, result, raw }` | works |
| `invoke --chain <key> --to <contract> (--sig "<fn>" [ARGS...] \| --data 0x...) [--value <dec>] [--dry-run] [--wait] [--rpc <url>]` | transfer shape + `{ sig, args, data, value }` | `WALLET_LOCKED` |
| `transfer --chain <key> --to <0x> --amount <dec> [--token <erc20>] [--dry-run] [--wait] [--rpc <url>]` | see below | `WALLET_LOCKED` |
| `sign-message [--chain <key>] --message <s>` | `{ chain, key, address, message, signature, digest }` (EIP-191) | `WALLET_LOCKED` |
| `unlock <dur> [--password-stdin]` | `{ locked: false, duration, expiresAt, active, address }` | - |
| `lock` | `{ locked: true, wasUnlocked }` | - |
| `backup` | prints the recovery phrase; interactive terminal only | asks for the password |

- **Durations** are positional: an integer and one unit of `s m h d w y` (`1y` = 365d). `0`,
  negatives, fractions and unknown units are `VALIDATION_ERROR`.
- **`create <name>`**: on first run it creates the master seed (BIP39, 24 words by default) and key
  `<name>` at index 0. Afterwards it derives the next index as `<name>` and makes it active. While
  unlocked this needs no password (the daemon derives); while locked it asks for the password for
  that one operation and starts no daemon.
- **`call` / `invoke` signatures** are `name(argTypes)(returnTypes)`, e.g.
  `balanceOf(address)(uint256)` or `getReserves()(uint112,uint112,uint32)`. Return types are
  optional (without them `result` is null and `raw` has the hex). Arrays are written `[1,2]`,
  tuples `(a,b)`; scalar results are strings.
- **`transfer` / `invoke`** fill the pending nonce, EIP-1559 fees (legacy `gasPrice` where the chain
  has no base fee), `eth_estimateGas` + 20% (exactly 21000 for plain transfers) and, on Base and OP
  Mainnet, the L1 data fee. Without `--dry-run` a shortfall is `INSUFFICIENT_FUNDS` before anything
  is signed. `--dry-run` signs but does not broadcast and returns `{ dryRun: true, chain, key, from,
  to, amount, nonce, gasLimit, type, maxFeePerGas, maxPriorityFeePerGas, gasPrice, maxGasCost,
  l1Fee, sufficientFunds, shortfall, signerDecoded, rawTx, txHash, recoveredFrom }`;
  `recoveredFrom` is recovered from the signed raw bytes, `signerDecoded` is what the key daemon
  decoded from the bytes it signed, and an empty wallet can be dry-run. Live: `{ ..., txHash, explorerUrl }`
  plus `{ status, blockNumber, gasUsed }` with `--wait` (`status: "pending"` if not mined within 5
  minutes; a receipt problem never turns a broadcast into an error).
- **Amounts** are decimal strings converted exactly (no floats); more fractional digits than the
  asset's decimals is `VALIDATION_ERROR`. ENS names are refused (`INVALID_ADDRESS`).
- **Error codes**: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`, `WRONG_PASSWORD`,
  `WALLET_LOCKED`, `KEY_EXISTS`, `KEY_NOT_FOUND`, `CHAIN_UNSUPPORTED`, `CAPABILITY_UNSUPPORTED`,
  `INVALID_ADDRESS`, `RPC_ERROR`, `INSUFFICIENT_FUNDS`.

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
default. zwallet checks the endpoint's `eth_chainId` before using it.

## Custody model

- **Home**: `$ZWALLET_HOME` or `~/.zwallet`, mode 0700.
- **`wallet.json`** (0600, written atomically): a sealed section encrypted with AES-256-GCM under a
  scrypt key (N=2^15, r=8, p=1; parameters, salt and nonce stored in the file), plus non-secret
  metadata: `createdAt`, `active`, `nextIndex`, and
  `keys: [{ name, index, createdAt, addresses: { <chain>: <address> } }]`. The sealed plaintext is
  a versioned object `{ version, mnemonic, policy }`: one BIP39 English mnemonic (no BIP39
  passphrase) and a `policy` object reserved for a future signing policy (empty for now, carried
  through intact). The policy lives under the password because anything outside it can be edited
  by the same OS user without the password.
- **Password**: held by the human only. There is no password file and no password env variable;
  it is typed at a terminal prompt (no echo) or piped with `--password-stdin`. Nothing on disk
  decrypts the wallet by itself. `create` reads `wallet.json` back from disk, decrypts it and checks
  the addresses before reporting success, and removes only what it created on failure.
- **Keys**: named subkeys of the master by index, in creation order, never reused. EVM uses
  `m/44'/60'/0'/0/<index>` (the MetaMask/Trust account order), so the phrase imports into standard
  wallets with the same accounts. One address per key on every EVM chain.
- **Daemon**: `unlock` starts `zwallet daemon` detached (own session) and sends it the password
  over `$ZWALLET_HOME/d.sock` (0600; peer uid checked on both ends). The daemon decrypts the wallet
  itself, keeps only the master seed in memory, and does nothing but derive addresses and sign; it
  makes no network request. For a transaction it receives only the unsigned signing payload bytes,
  decodes them itself (type, chain id, recipient, value, and ERC-20 `transfer` / `approve` /
  `transferFrom` from the calldata), refuses anything zwallet does not produce (other tx types,
  access lists, contract creation, non-canonical encodings), and signs exactly the bytes it decoded.
  It never trusts intent fields from the CLI, which is the agent-controlled side. On `lock`, at expiry (wall clock), or on SIGTERM it wipes the seed and
  exits. A reboot or a dead daemon means locked. Everything else (RPC, ABI, tx building, broadcast)
  runs in the one-shot CLI process, which checks every signature it gets back by recovering the
  signer.
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
