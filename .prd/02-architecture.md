# zwallet — Architecture

Status: **in-progress**, 2026-10-20. This is the target for v0.1. Sections move to the present tense
when the code that implements them ships.

## Stack

Rust, one binary `zwallet`. Crates: `alloy` (secp256k1 signer and mnemonic derivation, HTTP
provider, consensus and tx types, `sol!` ERC-20, dyn-abi/json-abi for `call`/`invoke` signatures),
`clap` (derive), `tokio`, `serde`/`serde_json`, `scrypt`, `aes-gcm`, `rand`, `zeroize`. No external
binaries (no foundry `cast`/`anvil`, including in tests).

## Modules

```text
src/
  main.rs            CLI (clap) → dispatch
  core/              chain-agnostic: store (wallet.json), seed, amounts (decimal ↔ base units,
                     any decimals), durations, errors + JSON output, daemon protocol + client
  daemon/            the key holder (hidden `zwallet daemon` subcommand)
  chains/mod.rs      registry (key → family + family config) and the family trait
  chains/evm/        everything EVM: derivation, RPC, tx build, ERC-20, ABI call/invoke
```

**Family trait**: derive address from seed; native balance; token balance; build an unsigned
transfer; sign (in the daemon); broadcast; sign a message. `call`/`invoke` are an EVM capability.
A family without a capability returns `CAPABILITY_UNSUPPORTED`. Registry rows carry
`family: Evm` plus EVM config (chainId, default RPC, explorer). Later rows will carry `Solana` or
`Ton` with their own config.

**Extension points (not implemented in v0.1)**: Solana = ed25519 SLIP-0010 `m/44'/501'/0'/0'`.
TON = ed25519 `m/44'/607'/0'` plus a wallet-contract address (v4R2/v5R1). Both derive from the same
seed, so the wallet file and the daemon protocol do not change.

## Custody

- **Home**: `$ZWALLET_HOME` or `~/.zwallet`, mode 0700.
- **`wallet.json`** (0600, atomic write): versioned, chain-agnostic. Contains the master mnemonic
  (24 words by default) encrypted with AES-256-GCM, a key from scrypt with the parameters stored in
  the file, and non-secret key metadata:
  `keys: [{ name, index, createdAt, addresses: { <chain>: addr } }]` and `active`.
- **Keys**: a key = an index `i` under the master, assigned in creation order and never reused.
  Each family maps `i` to its own path. EVM uses `m/44'/60'/0'/0/i` (MetaMask/Trust account order).
  Solana will use `m/44'/501'/i'/0'` and TON `m/44'/607'/i'`. Every command acts on the active key
  unless `--key <name>` is given. `create <name>` on an existing master derives the next index
  through the unlocked daemon, or through a one-off password prompt.
- **Password**: held by the human only. No password file and no env variable. Typed at a TTY prompt
  or piped with `--password-stdin` (scripts and tests). `create` writes the file, reads it back and
  decrypts it, and checks the address before it reports success.
- **`wallet backup`**: prints the mnemonic only when stdin and stdout are TTYs, after the password is
  entered again. Otherwise it refuses, so an agent's shell never sees it.

## Process model: the daemon holds only the key

- `zwallet unlock <dur>` starts `zwallet daemon` detached (setsid, so it survives the terminal) if it
  is not running. It polls for the socket with a bounded total wait, never one fixed sleep. Then it
  sends `unlock{password, ttl}` over the socket. The daemon decrypts `wallet.json` itself. The
  password and seed never travel through argv or env.
- **The daemon holds the seed in memory**, zeroizes it on `lock` or expiry, and exits at expiry.
  **It does no RPC.** Requests: `unlock`, `lock`, `status`, `address{chain,index}`,
  `sign_message{chain,index,msg}`, `sign_tx{chain,index,unsigned tx}`. These are family-agnostic.
- **Socket** `$ZWALLET_HOME/d.sock`, 0600. The daemon checks that the peer uid matches its own
  (getpeereid / SO_PEERCRED). If the path is longer than the `sun_path` limit (~104 bytes on macOS),
  zwallet fails loudly with `VALIDATION_ERROR` rather than failing silently.
- **The CLI does everything else** in one process: RPC (nonce, EIP-1559 fees with a legacy
  fallback, `eth_estimateGas`, balance pre-check, broadcast, receipt polling), ABI encoding and
  decoding, and output.
- **Locked matrix**: `address`, `addresses`, `balance`, `call` and `status` work while locked
  (they use cached addresses). `sign-message`, `transfer` and `invoke` → `WALLET_LOCKED`, with the
  hint `zwallet unlock 24h`.
- A reboot or a dead daemon means the wallet is locked. This is by design. There is no autostart.

## Resistance (quantum / AI): decided 2026-10-20

The user asked for this "from the start, if usable" (ssot 18). Evidence:
[`research/01-pq-ai-resistance.md`](research/01-pq-ai-resistance.md).

**Quantum: not usable today, so zwallet is PQ-ready instead of PQ.**
- Every Ethereum PQ signature EIP is Draft: [EIP-8051](https://eips.ethereum.org/EIPS/eip-8051)
  ML-DSA, [EIP-8052](https://eips.ethereum.org/EIPS/eip-8052) Falcon,
  [EIP-7932](https://eips.ethereum.org/EIPS/eip-7932).
- Fusaka (mainnet 2025-12-03) added only P-256. Hegotá's [EIP-8141](https://eips.ethereum.org/EIPS/eip-8141)
  has no mainnet date.
- The only PQ smart account, [Kohaku](https://github.com/ethereum/kohaku/tree/3044423edf212554284128b64355f0d38db3863a/packages/pq-account),
  is testnet-only and unaudited, and its verifier READMEs warn "LOSS OF FUND WILL OCCUR".

PQ-readiness in v0.1:
1. **Keys are derived from a BIP39 seed.** Vitalik's quantum-emergency hard-fork proposal recovers
   funds for seed-derived keys with a STARK proof.
2. **The family/scheme boundary stays.** A hybrid ECDSA+PQ account becomes another account type
   behind the same key index when either an audited verifier and factory are on a target chain's
   mainnet, or EIP-8141 plus a PQ precompile activates.
3. **Rotation is on demand.** `zwallet create <name>` gives a never-signed address, whose public key
   is unexposed.

Spend-once addresses are **not** the default. They break the stable deposit address the user asked
for, and an EOA cannot sweep ERC-20 and ETH atomically.

**AI resistance: usable, in v0.1, as a signing policy enforced by the daemon (WU1b).**
- The policy lives inside the encrypted, authenticated part of wallet.json, under the
  password-derived key. Viewing it works while locked; changing it needs the password, which only
  the human holds.
- In `sign_tx` the daemon decodes the tx and the calldata itself and never trusts CLI intent
  fields, because the CLI is the process a prompt injection controls.
- Defaults (the human confirms or edits them at `create` on the TTY):
  - Native spend is allowed under a **per-period cap** (rolling 24 h), never a per-tx cap alone.
  - `approve` / `increaseAllowance` / `setApprovalForAll` / Permit2 selectors are **denied** unless
    the spender is allowlisted, since that is the real drain vector.
  - ERC-20 transfers need a per-token cap set by the human; an unknown token is denied.
  - Arbitrary `invoke` is allowed with native value under the cap.
  - `sign-message` is EIP-191 only, with no typed-data signing.

**Deferred hardening, with known limits stated:**
- Running the daemon as a separate OS user.
- A Secure Enclave biometric approval app for out-of-policy actions.
- Holding only child keys in daemon memory.

Until these exist, a process running as the same user can still swap the zwallet binary, fake the
unlock prompt to capture the password, or talk to the socket within policy. Root defeats everything
local. The policy stops a prompt-injected agent that uses zwallet as intended. It does not stop
malware that already runs as the user.

## CLI contract

The CLI is flat: `zwallet create <name> | list | change <name> | address | addresses | balance |
call | invoke | transfer | sign-message | status | backup | chains | lock | unlock <dur>`.
Every command takes `--json`. On failure the CLI prints `{ "error", "errorCode" }` on stdout and
exits 1. Codes: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`, `WRONG_PASSWORD`,
`WALLET_LOCKED`, `CHAIN_UNSUPPORTED`, `CAPABILITY_UNSUPPORTED`, `INVALID_ADDRESS`, `RPC_ERROR`,
`INSUFFICIENT_FUNDS`. Command shapes are in the README.

RPC endpoint precedence: `--rpc` flag, then `ZWALLET_RPC_<KEY>` env, then the default public node
(publicnode.com per chain).

## Distribution (the llmux/harn pattern)

- `ci.yml`: fmt, clippy `-D warnings`, and test on `ubuntu-latest` and `macos-latest`.
- `release.yml`: on a `v*` tag, checks that the tag equals the `Cargo.toml` version. Builds for
  `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, and
  `aarch64-unknown-linux-gnu` (on `ubuntu-24.04-arm`). Publishes bare binaries named
  `zwallet-<os>-<arch>` plus `SHA256SUMS`.
- `preview.yml`: every push to `main` publishes the prerelease `preview-<YYYY-MM-DD-HHMM>-<sha12>`
  with the same assets.
- Version line: `zwallet <ver> (<channel> <rev12>)`, built from `ZWALLET_BUILD_CHANNEL` and
  `ZWALLET_BUILD_ID`. The preview formula test checks for the word `preview`.
- **`2lab-ai/homebrew-tap`**: `Formula/zwallet.rb.tmpl` and `zwallet-preview.rb.tmpl` (copies of
  harn's with the names changed). `bump.yml` jobs `bump-zwallet` and `bump-zwallet-preview` run on
  the 6-hourly schedule. Right after a release, the formula is rendered by hand with the same
  procedure (asset sha256 → template → commit → push) instead of waiting for the cron.
- **`2lab-ai/xbrew`**: `recipes/zwallet.toml`. `[macos] formula = "2lab-ai/tap/zwallet"`.
  `[arch.binary]` has immutable release URLs and sha256 for aarch64 and x86_64. Debian and RHEL
  fall back to brew.
- **Visibility**: the tap downloads release assets without authentication, so the repo becomes
  public at the first release, as llmux, harn and xbrew are.
