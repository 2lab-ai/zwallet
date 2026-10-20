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
- **Password**: held by the human only, at least 12 characters. There is no password file and no
  env variable. It is typed at a TTY prompt or piped with `--password-stdin` (scripts and tests),
  and read with raw `read(2)` into a `Zeroizing` buffer. Core dumps are disabled (and on Linux the
  process made non-dumpable) on the first line of `main_entry`, for every command.
- **KDF**: scrypt `log_n = 18`, `r = 8`, `p = 1` (256 MiB). On decrypt, only `r = 8`, `p = 1` and
  `15 ≤ log_n ≤ 18` are accepted, checked before anything is allocated. A tampered header cannot
  demand gigabytes.
- **Writes**:
  - `create` writes a temp file, then decrypts it again and verifies it, then publishes with
    `link(2)` (no-clobber; there is no rename fallback).
  - After publication, a later failure never unlinks `wallet.json`; the error names it for recovery.
  - Every metadata write (create, add_key, change) holds an exclusive `flock` on `w.lock` across
    read → plan → publish. A new key's index is `max(nextIndex, highest index + 1)`. Unlock rejects
    duplicate indices or names.
  - Directory fsync errors propagate.
- **`wallet backup`**: prints the mnemonic only when stdin and stdout are TTYs, after the password is
  entered again. Otherwise it refuses, so an agent's shell never sees it.

## Process model: the daemon holds only the key

- **Unlock** (staged; review rounds 1–3, 2026-10-20). `zwallet unlock <dur>` never sends the
  password over the socket: in round 1, a same-user listener on `d.sock` received it.
  1. Linux only: the CLI refuses when `kernel.yama.ptrace_scope` is 0 or missing, because then any
     same-user process can attach to the daemon during start-up. `--allow-ptrace-scope-0`
     overrides this (needed in containers without Yama).
  2. Before prompting, the CLI pins the daemon executable. Linux spawns `/proc/self/exe`; macOS
     records the (dev, ino) of `current_exe` and re-checks it right before the spawn. macOS has no
     `fexecve`, so a swap racing check→exec is part of the binary-swap limit.
  3. It spawns its **own** `zwallet daemon --home <canonical home>` child, with setsid. Every other
     open fd is marked close-on-exec: macOS lists them with `PROC_PIDLISTFDS`, Linux uses
     `close_range` and falls back to `/proc/self/fd`. Concurrent unlocks take turns on `u.lock`.
  4. The channel is one end of a `UnixStream::pair()` as the child's fds 0 and 1, not pipes. In
     round 3, a pipe was shown to be reopenable through `/proc/<pid>/fd` before the child turns
     non-dumpable; a socket returns ENXIO. The Linux daemon refuses to start unless fds 0 and 1
     are sockets.
  5. The child says `{"stage":"input"}`. Only after that does the CLI write the TTL and password.
  6. The child decrypts **without** `d.lock` and says `decrypted`. A wrong password stops here and
     leaves any running session untouched.
  7. The CLI stops this home's old daemons. It sends a polite socket `Lock`, then uses the
     process table.
  8. It sends a go byte. The child takes `d.lock` (5 s wait), unlinks `d.sock`, binds, and writes
     `{expiresAt, active, index, address, pid}`, with the address derived from the seed. If that
     write fails because the CLI died, the child wipes the seed and exits.
  9. The CLI reports success only after that line arrives. On an error or timeout it kills its child.

  The password and seed never travel through argv, env, or the socket.
- **Lock by process table, not by filename** (rounds 2–3). A same-user process can rename the real
  socket and put a fake one at `d.sock`, replace `d.lock`, or repoint a symlink in the home path.
  - `zwallet lock` is the panic button. It finds **every** process of this uid whose argv is
    `<any> daemon --home <absolute path>`, whatever the home (macOS `KERN_PROCARGS2`, Linux
    `/proc/<pid>/cmdline`). It SIGTERMs them and SIGKILLs any still alive after 2 s. It reports
    `stopped: [pids]`, and succeeds only when none remain.
  - `unlock` stops only its own home's daemons. It matches by name first, then by inode. The stats
    run on helper threads, bounded to 1 s and cached, so a FUSE path in another process's argv
    cannot hang it.
  - `status.locked` comes from the process table (`daemonPids`), not from the socket's answer.

  A fake process that copies that argv gets killed too, which is harmless.
- **The daemon holds the seed in memory**, zeroizes it on `lock` or expiry, and exits at expiry.
  **It does no RPC.** Socket requests (protocol v2): `lock`, `status`, `address{chain,index}`,
  `sign_message{chain,index,msg}`, `sign_tx{chain,index,unsigned tx}`. These are family-agnostic.
  There is no `unlock` request.
- On every tick (≤1 s) and on every request, the daemon checks:
  - expiry;
  - that `wallet.json`'s ciphertext is unchanged;
  - that `d.sock` is still its own socket (dev/ino).

  Any failure wipes the seed and exits. Every `wallet.json` read opens with
  `O_NOFOLLOW|O_NONBLOCK` and requires a regular file of at most 1 MiB owned by our uid, so a FIFO
  cannot hang the tick. Each connection is served in its own task with a 10 s I/O
  timeout, so a silent client cannot hold the seed past expiry.
- **Socket** `$ZWALLET_HOME/d.sock`, 0600. The daemon checks that the peer uid matches its own
  (getpeereid / SO_PEERCRED). If the path is longer than the `sun_path` limit (~104 bytes on macOS),
  zwallet fails loudly with `VALIDATION_ERROR` rather than failing silently.
- **The CLI does everything else** in one process: RPC (nonce, EIP-1559 fees with a legacy
  fallback, `eth_estimateGas`, balance pre-check, broadcast, receipt polling), ABI encoding and
  decoding, and output.
- **Address trust** (round 2): the address cache in `wallet.json` is not authenticated, and it
  decides where funds go. A same-user process can also swap in its own wallet and run a genuine
  daemon with its own password.
  - So **`verified: true` only follows the human's password, or a seed generated in this process**:
    - `address --verify` and `list --verify` (decrypt locally, derive, compare);
    - the readiness line of `unlock` and `create --unlock`;
    - the first `create`;
    - a subkey `create` at a terminal or with `--password-stdin`.
  - Everything else says `verified: false`, with "run `zwallet address --verify` before funding".
    That covers an agent's `create` through the daemon, and `balance`.
  - The daemon cross-check remains as a tamper detector only: a mismatch is a VALIDATION_ERROR, and
    a match is not verification.
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

A Homebrew-installed binary is writable by the user, so the binary-swap limit applies to it in
full. Real protection means installing zwallet where the agent cannot write, or running the agent
as a different OS user (README §Known limits).

Until these exist, a process running as the same user can still swap the zwallet binary, fake the
unlock prompt to capture the password, or talk to the socket within policy. Root defeats everything
local. The policy stops a prompt-injected agent that uses zwallet as intended. It does not stop
malware that already runs as the user.

## CLI contract

The CLI is flat: `zwallet create <name> | list | change <name> | address | addresses | balance |
call | invoke | transfer | sign-message | status | backup | chains | lock | unlock <dur>`.
Every command takes `--json`. On failure the CLI prints `{ "error", "errorCode", …details }` on
stdout and exits 1. Codes: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`,
`WRONG_PASSWORD`, `WALLET_LOCKED`, `KEY_EXISTS`, `KEY_NOT_FOUND`, `CHAIN_UNSUPPORTED`,
`CAPABILITY_UNSUPPORTED`, `INVALID_ADDRESS`, `RPC_ERROR`, `INSUFFICIENT_FUNDS`,
`BROADCAST_UNCERTAIN`. Command shapes are in the README.

**Send safety** (review rounds 1–2). No message or code may nudge an agent into a second send of
the same payment.
- **One send per key and chain at a time.** A send holds `nonce-<chainId>-<addr>.lock` from before
  `prepare` until the command returns, including `--wait` (`--wait-timeout`, default 90 s). A
  second send while the lock is held fails at once with nothing signed.
- **Send journal** (round 3). Right before `eth_sendRawTransaction`, one line is appended and
  fsynced to `sends-<chainId>-<addr>.jsonl` (0600): `{ts, chain, from, nonce, txHash, to, value,
  kind, token?}`. The same fields go to stderr as "broadcasting …". If the journal write fails,
  nothing is broadcast. A process killed after the broadcast therefore still leaves a record.
- **Fresh-send guards** (round 3; a send without `--nonce`, live only; a dry-run reports instead):
  - **Pending guard:** refused if the account's pending nonce exceeds its mined nonce, i.e. a tx
    from this key is still in the pool. The error names the pending nonces and their journal
    entries. `--allow-pending` overrides.
  - **Duplicate guard:** refused if the journal has the same chain, kind, `to`, value (and token)
    from the last 10 minutes. This catches "my first attempt was killed, re-run" even when that
    attempt has already been mined. `--allow-duplicate` overrides.
- **Before the send:** an error is `RPC_ERROR` "nothing was broadcast — if you retry, pass
  `--nonce N`".
- **After the send:** the computed hash is polled.
  - If it is known, that is success (with `broadcastWarning` / `rpcTxHash` where relevant).
  - "Nonce too low" and "replacement underpriced" are looked up by hash first. If not found, the
    result is "nonce N is taken by another transaction": `BROADCAST_UNCERTAIN` if `--nonce` was
    given, otherwise `RPC_ERROR`. The refused hash goes in `refusedTxHash`, never `txHash`, and
    the message points at the earlier send's hash (and its journal entries). It never says "next
    nonce".
  - Plain "transaction underpriced" (below the pool minimum) is a definitive rejection.
  - A wrong hash returned by the RPC is reconciled by the computed hash, falling back to
    `BROADCAST_UNCERTAIN`.
  - `--nonce` below the mined count says the nonce is already mined and tells the agent to look up
    that send. It never suggests "the next free nonce".
- **Replacement:** `--nonce N` below the pending nonce requires `--replace <txHash>`, so a retry
  can never silently evict a different payment. `--bump <1.1–3>` (default 1.25) always applies
  when given. `--replace` bumps from the original's fees. Every hint after a send says
  `--nonce N --replace <txHash>`. A `--wait` timeout prints a hint that a run without `--nonce`
  is a second payment.
- `--dry-run` never prints the signed raw tx.
- **Fee ceiling**, unless `--max-fee-gwei` is given:
  - EIP-1559: tip ≤ 2×reference and price ≤ 6×reference, where reference = `max(baseFee,
    eth_gasPrice, 1 gwei)`.
  - Legacy chains (BSC): price ≤ `6 × max(baseFee, 1 gwei)`.
  - These ceilings trust the RPC. An RPC-independent absolute fee budget belongs in the daemon
    policy (WU1b).
- **Daemon decode:** the actions are method-shaped (`transfer-call` / `approve-call` /
  `transfer-from-call`) with `assetType: "unverified"`, because ERC-721 shares the `approve` and
  `transferFrom` selectors. Known selectors must re-encode byte-for-byte and carry `value = 0`.
- **ABI input:**
  - `invoke` address arguments are EIP-55 checked.
  - Integers must be plain decimal or `0x` hex.
  - `bytes*` and `function` values need `0x`.
  - Top-level strings are taken verbatim.
- **RPC errors:**
  - URLs are redacted, including key-like host labels.
  - HTTP error bodies are never printed.
  - serde errors print only a fixed text with the line and column.
  - JSON-RPC `data` is printed only when it is a string, and then scrubbed.
  - Human-mode errors print `txHash`/`refusedTxHash`/`nonce`/`from`/`journal`.

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
