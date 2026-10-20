# Research 01 — post-quantum and "AI-resistant" wallet options (as of 2026-10-20)

Status: **reference** — research note, 2026-10-20, written by a research subagent from primary
sources (links pinned to commits where possible). It is an input to
[`../02-architecture.md`](../02-architecture.md) §Resistance, not a contract. Items marked
[unconfirmed] were not confirmed from a primary source.

## Bottom line

- No post-quantum (PQ) account can hold real funds on Ethereum or on any of the target L2s under
  zwallet's constraints (no third-party login/API key, public RPC). No fork has activated a PQ
  verifier, no L2 has shipped one, and the only PQ smart account (Kohaku / ZKNox) is testnet-only
  and unaudited.
- "AI-resistant" has no standard definition. In practice it means a wallet that a manipulated agent
  cannot drain. Policy enforced by the key-holding daemon before signing is usable today. It is
  only as strong as the OS boundary around the daemon.

## Verdict table

| Option | Usable today? | Why | Source |
|---|---|---|---|
| PQ precompile / native PQ EOA on L1 | No | Fusaka (mainnet 2025-12-03) added only P-256 (EIP-7951); every PQ EIP is Draft | [EIP-7607](https://github.com/ethereum/EIPs/blob/3b1edfd68cdfa6710510f52dbf1f6035b53e085b/EIPS/eip-7607.md), [EIP-8081](https://github.com/ethereum/EIPs/blob/6e69e49a99204fc7686b4975e95a9ac9ddb4fe7a/EIPS/eip-8081.md) |
| PQ precompile on an L2 | No | No PQ proposal in RIPs; RIP-7212 is P-256 | [RIPs@bd9b89f0](https://github.com/ethereum/RIPs/tree/bd9b89f0b02d26a579ef8972431bc93540314dd4/RIPS) |
| EIP-8141 frame transactions | No (future) | Scheduled for Hegotá, which has no mainnet date; Draft | [EIP-8141](https://eips.ethereum.org/EIPS/eip-8141) |
| Kohaku hybrid ERC-4337 account (ECDSA AND ML-DSA/Falcon) | Experimental | Testnets only, unaudited, 1.69M–8.39M gas per op | [kohaku@3044423e](https://github.com/ethereum/kohaku/tree/3044423edf212554284128b64355f0d38db3863a/packages/pq-account) |
| PQ verifier behind EIP-7702 | No | The EOA's ECDSA key keeps control | [EIP-7702](https://eips.ethereum.org/EIPS/eip-7702) |
| Self-submitting `handleOps` from our own EOA | Yes | EntryPoint v0.7 `handleOps` is callable by anyone | [v0.7.0 L174-177](https://github.com/eth-infinitism/account-abstraction/blob/v0.7.0/contracts/core/EntryPoint.sol#L174-L177) |
| Spend-once EOAs | Yes | The public key stays hidden until the first signature | [ethereum.org](https://ethereum.org/roadmap/security/quantum-resistance/) |
| Algorand Falcon-1024 accounts | Yes (later family) | Native in go-algorand v5.0.0-stable (2026-08-12) | [release](https://github.com/algorand/go-algorand/releases/tag/v5.0.0-stable) |
| NEAR ML-DSA-65 access keys | Yes (later family) | Stabilized in nearcore 2.13.0 (2026-07-09); activation date [unconfirmed] | [release](https://github.com/near/nearcore/releases/tag/2.13.0) |
| QRL 2.0 (Zond) | No | Testnet only | [QRL weekly](https://www.theqrl.org/weekly/2026-august-14/) |
| Policy enforced in our daemon | Yes | Checked before signing; bounded by the OS boundary | — |
| On-chain policy (Safe allowance module, Coinbase spend permissions) | Yes, but conflicts with spend-once | Audited, but needs a fixed ECDSA-owned contract | [Safe modules](https://github.com/safe-global/safe-modules/tree/9e5f3d0701c4bd5d5fa4ce8ff7d5206f2ccc4acc/modules/allowances) |
| Turnkey / Privy policy engines | No | A hosted third party | constraint |

## Q1 — native PQ on Ethereum and L2s

- **Fusaka:** EIP-7607 (Final) records mainnet activation on 2025-12-03. Its only signature change
  is EIP-7951 (secp256r1/P-256).
- **Glamsterdam:** live on Sepolia 2026-10-06; Hoodi 2026-10-26; the mainnet date has not been
  decided. None of its 18 EIPs touch signatures
  ([EF post](https://blog.ethereum.org/2026/10/16/glamsterdam-hoodi-announcement)).
- **PQ EIPs, all Draft:** [EIP-8051](https://eips.ethereum.org/EIPS/eip-8051) ML-DSA (2025-10-15,
  0x12/0x13, 4,500 gas); [EIP-8052](https://eips.ethereum.org/EIPS/eip-8052) Falcon (2025-10-17,
  3,000 gas); [EIP-7932](https://eips.ethereum.org/EIPS/eip-7932) signature registry (2025-04-12);
  EIP-7619 and EIP-8355 (EIP-8355 is on Hegotá's Declined list).
- **Hegotá (EIP-8081, 2026-10-19):** scheduled for inclusion are EIP-7805 (FOCIL) and EIP-8141 (frame
  transactions), with no mainnet date. EIP-8141 ships SECP256K1, P256 and an "ARBITRARY" scheme and
  names no PQ scheme. It also says 7702 authorization "does not satisfy the PQ goals".
- **ethereum.org** (updated 2026-09-08) says "Right now: nothing" for accounts; core PQ
  infrastructure targeted around 2029.
- **L2s:** no shipped PQ verifier was found. This is a negative search result, not an exhaustive
  check [unconfirmed].

## Q2 — PQ smart accounts

- **Kohaku `pq-account`** (ZKNox, MIT): `isValid` requires both the classical and the PQ signature
  ([L46-71](https://github.com/ethereum/kohaku/blob/3044423edf212554284128b64355f0d38db3863a/packages/pq-account/src/ZKNOX_ERC4337_account.sol#L46-L71)).
  Keys are fixed in the constructor, with no rotation and no ERC-1271.
  [deployments.json](https://github.com/ethereum/kohaku/blob/3044423edf212554284128b64355f0d38db3863a/packages/pq-account/deployments/deployments.json)
  lists Sepolia, Arbitrum Sepolia and Base Sepolia only.
- **Audit:** none in the repo. The [ETHFALCON](https://github.com/ZKNoxHQ/ETHFALCON) and
  [ETHDILITHIUM](https://github.com/ZKNoxHQ/ETHDILITHIUM) READMEs say "experimental work, not
  audited: DO NOT USE IN PRODUCTION, LOSS OF FUND WILL OCCUR".
- **Gas per op:** k1+ETHFALCON 1.69M, k1+FALCON 4.06M, k1+ML-DSA-ETH 5.12M, k1+ML-DSA 8.39M
  ([README](https://github.com/ethereum/kohaku/blob/3044423edf212554284128b64355f0d38db3863a/packages/pq-account/README.md)).
- **No bundler key needed:** EntryPoint v0.7 at `0x0000000071727De22E5E9d8BAf0edAc6f37da032`
  accepts `handleOps` from any caller. v0.9 requires a plain, non-delegated EOA caller
  ([L68-74](https://github.com/eth-infinitism/account-abstraction/blob/v0.9.0/contracts/core/EntryPoint.sol#L68-L74)).

## Q3 — what a quantum attacker gets from a plain EOA

- Before the first signature, only the address hash is public. After any signature (a tx,
  personal_sign, EIP-712, a permit, a 7702 authorization), the public key can be recovered.
- The same key gives the same address on every EVM chain, so one send on Base exposes the address
  everywhere.
- **Spend-once mitigation:** each address signs once and sweeps the remainder to a fresh address.
  ERC-20s need an atomic batch, which an EOA cannot do in one tx. Cost: an extra sweep per asset
  and no persistent address (no ENS, approvals or DeFi positions).
- **Seed-derived keys matter:** Vitalik's
  [emergency hard-fork proposal](https://ethresear.ch/t/how-to-hard-fork-to-save-most-users-funds-in-a-quantum-emergency/18901)
  (2024-03-09) recovers funds of keys derived from a seed by hashing, using a STARK proof.
- Per [BIP-32](https://github.com/bitcoin/bips/blob/130b8e0fa7f30fa45532f080b87b8db27a79e46e/bip-0032.mediawiki#implications),
  a parent xpub plus any non-hardened child private key gives the parent private key. Keep xpubs
  inside the daemon.

## Q4 — non-EVM chains with native PQ accounts (for later families)

Algorand: Falcon-1024 accounts in go-algorand v5.0.0-stable (2026-08-12). NEAR: ML-DSA-65 in
nearcore 2.13.0. QRL Zond: testnet. Solana: only single-use Winternitz vaults; the SIMD-0461 Falcon
proposal was closed on 2026-09-01. TON: nothing found.

## Q5 — "AI-resistant wallet"

- The threat is real: on 2026-05-04 a Morse-code prompt drove Grok and Bankrbot to move about
  $150–200k on Base ([OECD](https://oecd.ai/en/incidents/2026-05-04-4a73)).
- **What can be enforced locally:** the daemon can apply per-period caps, recipient allowlists,
  contract and method allowlists (with calldata decoding), approval thresholds, and scoped expiring
  sessions, all before signing.
- **Policy gaps to cover:** `approve`, Permit2 and `setApprovalForAll` (the drain happens later in
  the attacker's own tx); recipients inside router calldata; off-chain typed-data signatures;
  per-tx caps losing to many small txs (use per-period caps).
- **Limits of daemon-side enforcement:**
  - A same-user process can edit files, swap the binary, talk to the socket, or fake the unlock
    prompt.
  - On Linux with Yama `ptrace_scope=0`, any same-user process can attach.
  - The macOS hardened runtime only helps if the binary is signed that way.
  - Root defeats everything local.
- **The research agent's stronger proposal:** run the daemon as a separate OS user with a
  default-deny allowlist and per-period caps. Out-of-policy actions and policy changes would need a
  Secure Enclave P-256 key gated by biometry, held in a separate approval app that shows the decoded
  tx. The Secure Enclave holds only P-256 keys, so it can gate the secp256k1 key but cannot hold it.
  The approval-app arrangement is the agent's own design, not sourced.
