# zwallet — Spec

`zwallet` is a wallet CLI that an AI agent runs on its own machine. It does four things: **create** a
wallet, read **balances**, **call contract functions** (read and write), and **transfer** tokens. Keys
are generated and kept on the machine. Chain access is plain JSON-RPC to public nodes. A human
**unlocks** the wallet for a duration they choose (`zwallet unlock 24h`, `365d`, `1y`). While it is
unlocked, a small daemon holds the key, so the agent can sign without the human present. A human
`zwallet lock` (or the end of that duration, or a reboot) locks it again. EVM chains ship first;
Solana and TON are planned and the architecture leaves room for them.

Status: **in-progress**, 2026-10-20. Branch `feat/1-evm-wallet-core`. Fixed user statement and
scope: [`v0-1/ssot.md`](v0-1/ssot.md). Driver: [`v0-1/loop.md`](v0-1/loop.md).

## Problem

The user wanted an Ethereum address the agent can use on its own, funded with a small amount
(2026-10-20). Two existing agent wallet CLIs were measured that day and did not fit:

- **Trust Wallet `twak`** (`@trustwallet/cli` 0.20.0) runs `ensureTrustSDK() → getCredentials()` at
  the top of every `wallet` command. So `wallet create` fails with `No API credentials found` until
  the user signs up at portal.trustwallet.com (Google OAuth) and installs API keys. This happens even
  though key generation itself (`HDWallet.create(128, "")`) is local. It saves the wallet password to
  the OS keychain by default, and balance and transfer go through Trust's backend.
- **Tether WDK CLI** (`@tetherto/wdk-cli` 1.0.0-beta.7) creates keys locally and uses public nodes.
  But its signing daemon's session lasts `SESSION_TTL_MINUTES = 5` (`src/config/constants.js:51`),
  so an agent would need a human to unlock it every five minutes.

Both were research only; zwallet takes no code, design or dependency from either (user,
2026-10-21: "twak은 처음에 리서치할때만 쓴거고 전혀 관계없음"). It is its own Rust wallet CLI, with
no account, no API key, no telemetry and no OS keychain.

## Goals

- **G1 Create.** One HD master: a BIP39 mnemonic of 24 words by default (`--words` 12–24),
  generated locally and stored encrypted under a password the human chooses. Named subkeys are
  derived from it by index (`zwallet create <name>`; the newest becomes active). `zwallet list`
  and `zwallet change <name>` show and switch keys. No network call is needed to create.
- **G2 Balance.** Native coin and ERC-20 balance for the wallet's address on any configured chain.
  Without `--chain`, `balance` reads every network at once (native plus registry tokens; testnets
  last).
- **G3 Function call.** `call` = read-only `eth_call` with a signature such as
  `balanceOf(address)(uint256)`, returning decoded values. `invoke` = signed state-changing
  transaction from a signature + args, or raw calldata, with an optional native value.
- **G4 Transfer.** Native and ERC-20 transfers. `--dry-run` builds and signs from live chain state
  but does not broadcast. `--wait` waits for the receipt.
- **G5 Human unlock.** `zwallet unlock <duration>` with a positional duration (s, m, h, d, w, y;
  `1y` = 365d), and `zwallet lock`. A daemon holds **only the key** for that duration. Everything
  else runs in a one-shot CLI process.
- **G6 Chain families.** EVM is implemented. Solana and TON slot in later behind the same family
  boundary without changing the wallet file or the daemon protocol. Tron (TRX, TRC20 USDT) is the
  first such family, planned for v0.3: [`05-tron.md`](05-tron.md).
- **G8 Signed whitelist (v0.1, WU1b).** Funds leave only to entries of a list signed with a key
  separate from the seed, enforced by the daemon. Approvals are revoke-only, there is a per-chain
  fee cap, and `sign-message` is gated ([`04-whitelist.md`](04-whitelist.md)).
- **G9 TUI (v0.2).** `zwallet tui`: keys, warm-up, balance + last 10 tx, tx detail, assets →
  transfer to whitelisted recipients ([`03-tui.md`](03-tui.md)).
- **G7 Distribution.** The llmux/harn pattern: CI, preview prereleases on every `main` push, stable
  releases on `v*` tags, Homebrew formulae `2lab-ai/tap/zwallet` and `2lab-ai/tap/zwallet-preview`,
  and an `xbrew install zwallet` recipe.

## Non-goals

Swap, bridge, on/off-ramp, price alerts, DCA, x402, WalletConnect, ENS names, any third-party API
key, telemetry, the OS keychain, daemon autostart, hardware wallets, multiple accounts per wallet,
and Solana/TON implementations in v0.1.

## Acceptance (run → observe)

| ID | Run | Observe |
|---|---|---|
| A1 | derive from `test test test test test test test test test test test junk` | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` (m/44'/60'/0'/0/0) |
| A2 | `create a --password-stdin` in a temp `ZWALLET_HOME` | an address is printed; the master is 24 words; home is 0700 and `wallet.json` is 0600; no file on disk can decrypt the wallet without the password |
| A2b | `create b` (unlocked), `list`, `change a`, `sign-message --key b` | b is active at index 1 (`0x7099…79C8` for the test vector); `change a` switches the address; the signature recovers to b; a duplicate name → `KEY_EXISTS`; an unknown name → `KEY_NOT_FOUND` |
| A3 | while locked: `address`, `balance`, `call`, `list`, `change` | they work; `sign-message`, `transfer`, `invoke` → `WALLET_LOCKED` |
| A4 | `unlock 24h` / `365d` / `1y`, then `status`, then `lock` | `expiresAt` = now + the duration; after `lock`, signing → `WALLET_LOCKED` |
| A5 | `unlock 1s`, wait | after the time passes: `WALLET_LOCKED`, and the daemon process is gone |
| A6 | `sign-message` | recovering the signer from the signature and the EIP-191 digest gives the wallet address |
| A7 | `transfer --dry-run` on ethereum mainnet (0-value self-transfer) | `recoveredFrom == from`; nonce and fees come from the live chain |
| A8 | `call` USDC `decimals()(uint8)` on ethereum; `invoke --dry-run` | `6`; `recoveredFrom == from` |
| A9 | `backup` from a non-TTY shell | refused; the mnemonic is never printed to an agent |
| A9b | the signed whitelist ([`04-whitelist.md`](04-whitelist.md) W1–W12): a send to an unlisted payee, a non-zero approve, an unlisted contract or selector, a hand-edited list, a fee over the cap | the daemon refuses with `WHITELIST_DENIED` / `WHITELIST_INVALID` / `FEE_CAP_EXCEEDED`; editing the list needs the password. (Amount caps were deferred on 2026-10-20: 04 §Not in v0.1.) |
| A10 | `brew install 2lab-ai/tap/zwallet` and `…/zwallet-preview` | `zwallet --version` shows the release version and the preview channel respectively |
| A11 | `xbrew install zwallet` | installs it (macOS through the tap formula; Arch through the verified release binary) |
| A12 | the user creates and unlocks the real wallet on fable-m5max | the agent reads the address and balance without the human present, and after funding broadcasts a transfer with a receipt |
