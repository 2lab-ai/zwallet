# zwallet

A local EVM wallet for AI agents, controlled by a human. Create keys on your machine,
choose which recipients and contracts the wallet may pay, then unlock signing for a
limited time. Your agent uses the CLI and JSON output; you keep the password.

zwallet is written in Rust and runs on macOS and Linux. It needs no wallet-service
account, API credentials, or OS keychain. Balances and transactions use public RPC
endpoints by default; you can supply your own.

## What you can use today

These capabilities describe the code on `main`. Stable releases can lag behind it;
`zwallet --version` shows your build's channel and revision.

| What you want to do | Available now | Boundary to know |
|---|---|---|
| Create and manage wallets | One encrypted BIP39 master seed, named derived keys, address verification and terminal-only recovery-phrase backup | No import/restore command in zwallet; the phrase can be imported into a compatible wallet |
| Check funds | Native and ERC-20 balances on eight EVM networks; all-network balance view | Automatic token listing uses the built-in registry, not discovery of every token you own |
| Send funds | Native coin and registry-token transfers, signed dry-runs, receipt waiting and transaction lookup | Requires an unlocked session and the daemon's signing policy |
| Use contracts | Read-only ABI calls; writes using ABI arguments or raw calldata | Writes need an allowed contract and selector; token approvals are revoke-only |
| Control an agent's signing | Timed unlock, immediate lock, signed recipient/contract whitelist, per-chain execution-fee caps | No spend-amount or daily budget caps yet; see the WIP items below |
| Integrate an agent | `--json`, structured error codes, send journal, pending/repeat guards and fixed-nonce replacement | A pending or unknown receipt is not permission to send the payment again |
| Inspect from a terminal UI | Key list, active-key marker, lock status, refresh and lock-all | The full wallet UI is still WIP |

### Planned features

![WIP](https://img.shields.io/badge/status-WIP-yellow) means **not available in the current implementation**, even if a design or
mockup exists. These are plans, not release-date promises. Existing CLI capabilities
remain available where the corresponding TUI screen is WIP.

| Feature | Status | What is still missing |
|---|---|---|
| Full terminal wallet UI | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Password warm-up/unlock, navigation and actions, balances/history, receive, send, whitelist editing and other CLI workflows in the TUI. `tui --warm` currently validates a duration but does not unlock. |
| Tron | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | TRX and TRC20 USDT addresses, balances, signing, transfers and transaction lookup; currently a v0.3 design only |
| Solana and TON | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Chain implementations; currently future extension plans |
| Spend-amount limits | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Rolling 24-hour native and per-token caps; today's fee cap only bounds execution gas per transaction |
| User-added tokens | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Adding transfer tokens outside the built-in registry |
| More precise repeat protection | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Calldata-aware `invoke` duplicate matching and a configurable duplicate window |
| Hardware-backed whitelist key | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | Moving whitelist authority to Secure Enclave or hardware |

See [feature status and code evidence](docs/features.md) for the current boundaries and
links to each plan.

## Install

Choose **one** installation method:

```sh
brew install 2lab-ai/tap/zwallet          # stable
# Or, for the current main-branch features:
brew install 2lab-ai/tap/zwallet-preview  # preview
```

Homebrew supports macOS and Linux. Both channels install `bin/zwallet`; uninstall the
other formula before switching.

You can also use `xbrew install zwallet`, build from source with Rust
(`cargo install --git https://github.com/2lab-ai/zwallet --locked`), or download a binary
and `SHA256SUMS` from [GitHub Releases](https://github.com/2lab-ai/zwallet/releases).
Binaries are provided for macOS and Linux on arm64 and x86_64. Verify the checksum,
make the binary executable, and place it on your `PATH` as `zwallet`.

Published macOS binaries are ad-hoc signed with the hardened runtime. An unsigned
local x86_64 build must be signed before `unlock` can start its daemon:

```sh
codesign --force --sign - "$(command -v zwallet)"
```

Check what you installed:

```sh
zwallet --version
zwallet --help
```

The version line includes the channel (`release`, `preview`, or `dev`) and revision
when build metadata is supplied.

## Quick start

Run the setup steps yourself in an interactive terminal. The agent does not need
your password. Start on Sepolia if you want to try a transfer with testnet funds.

### 1. Create and back up a key

```sh
zwallet create main       # choose a password of at least 12 characters
zwallet backup            # displays the recovery phrase after your password
zwallet address --verify # verify the receiving address against the encrypted seed
```

Store the recovery phrase privately before funding the address. `backup` requires an
interactive terminal and is not JSON output for an agent. Keep a current copy of
`wallet.json` after whitelist changes too: the phrase alone does not restore your
whitelist policy. See [custody and recovery limits](docs/security.md).

### 2. Allow a recipient, then unlock

Replace `0xRECIPIENT_ADDRESS` with the recipient's full EVM address before running:

```sh
zwallet whitelist add alice 0xRECIPIENT_ADDRESS --chains sepolia
zwallet unlock 30m
```

A whitelist edit asks for your password and ends existing signing sessions, so unlock
**after** editing the list. A listed recipient can receive the entire balance:
spend-amount caps are WIP. Unlocking also lets other processes under your OS user
request signatures within the policy.

### 3. Let the agent read and preview

```sh
zwallet balance --chain sepolia --json
zwallet transfer --chain sepolia --to @alice --amount 0.001 --dry-run --json
```

A dry-run uses live RPC data and, when allowed, signs without broadcasting. Check
`whitelist`, `sufficientFunds` and the transaction details. It does not expose a
broadcastable raw transaction. Read commands work while locked; signing needs the
unlocked daemon.

### 4. Send deliberately and check the result

After funding the verified address with Sepolia ETH and reviewing the dry-run:

```sh
zwallet transfer --chain sepolia --to @alice --amount 0.001 --wait --json
```

Save the returned `txHash`. If the result is pending or unknown, look up that hash
before retrying; replace `0xTRANSACTION_HASH` below with it:

```sh
zwallet tx --chain sepolia 0xTRANSACTION_HASH --json
zwallet lock
```

`lock` stops **every zwallet signing session for your OS user**, across wallet homes.
For retry and replacement rules, see [send safety](docs/commands.md#send-journal-and-retry-safety).

## Chains

| CLI key | Network | Native coin |
|---|---|---|
| `ethereum` | Ethereum | ETH |
| `base` | Base | ETH |
| `arbitrum` | Arbitrum One | ETH |
| `optimism` | OP Mainnet | ETH |
| `polygon` | Polygon PoS | POL |
| `bsc` | BNB Smart Chain | BNB |
| `avalanche` | Avalanche C-Chain | AVAX |
| `sepolia` | Sepolia testnet | ETH |

Run `zwallet chains --json` for chain IDs and endpoints. RPC selection is
`--rpc` → `ZWALLET_RPC_<CHAIN>` → built-in public endpoint. See
[RPC configuration](docs/commands.md#rpc-configuration) for details.

The same named key has the same address on all supported EVM networks. Balances and
whitelist permissions are still network-specific. Registry-token support varies by
network; BSC currently has no registry tokens for transfer.

## Terminal UI

```sh
zwallet tui --chain sepolia
```

Requires an interactive terminal of at least 80×24. Use `j`/`k` or arrow keys to
browse, `r` to refresh, `L` to lock all sessions, and `q` or Ctrl-C to quit. The current
UI shows keys and signing status; use the CLI for the remaining workflows. See
[the planned UI](.prd/03-tui.md) ![WIP](https://img.shields.io/badge/status-WIP-yellow).

## Before funding a wallet

- The whitelist limits recipients and contract selectors. It is not a per-payment
  approval screen, a daily budget, or isolation from processes under your OS user.
- Non-zero token approvals and `transferFrom` are refused. Token transfers use only
  registry deployments; reading an arbitrary token balance does not authorize sending it.
- Fee caps apply per transaction. On Base and Optimism, L1 data fees are outside that cap.
- Cached addresses can be edited by the same OS user. Verify the address before funding;
  Linux does not provide the same session-based address verification as macOS.

Read [security and custody](docs/security.md) for the trust model, rollback limits
and operating-system constraints.

## Documentation

| Reader | Start here |
|---|---|
| Agent integrator or CLI user | [Command reference, JSON fields and errors](docs/commands.md) |
| Wallet owner or security reviewer | [Security and custody](docs/security.md) |
| Evaluator or contributor | [Feature status and implementation evidence](docs/features.md) |
| Contributor or release maintainer | [Development and releases](docs/development.md) |
| Contributor working on future features | [Product specification](.prd/01-spec.md) and [architecture](.prd/02-architecture.md) |

Licensed under [MIT](LICENSE).
