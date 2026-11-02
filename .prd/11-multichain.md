# Shared-seed Solana and TON, explicit receive addresses
Status: implementation and local acceptance complete; final-head delivery receipt in [multichain/loop.md](multichain/loop.md).

## Intent and contract
2026-11-02: preserve the current BIP39 seed and existing EVM/Tron addresses. Add Solana and TON as first-class native-coin families: derive, password-verify/cache, receive address/QR, balance, allowlisted native send, transaction lookup, CLI and TUI. Display the selected Tron receive address separately on the same horizontal row as its QR when space permits, with a readable narrow-screen layout. No owner wallet or real funds are used for validation.

## Derivation and compatibility
Solana: BIP39 (empty passphrase) → SLIP-0010 Ed25519 at `m/44'/501'/{index}'/0'`.
TON: the same BIP39 seed → SLIP-0010 Ed25519 at `m/44'/607'/{index}'`, wallet V4R2, workchain 0, subwallet ID 698983191. This is multichain BIP39, not TON-native mnemonic derivation. Record derivation and contract identity for recovery. Retain current word counts/defaults and seal format; missing cached new-family addresses are filled only on an authenticated operation.

## Architecture and boundaries
Reuse FamilyKeys/ChainFamily, sealed seed, signed whitelist, immutable review/confirm, bounded RPC, send lock/journal, redacted output and typed capabilities. New chain code lives in `src/chains/solana` and `src/chains/ton`. Shared Ed25519 derivation and policy helpers live under chains, not daemon network code. The daemon strictly decodes the unsigned bytes and checks actual sender, recipient, amount, supported operation and signed policy before signing. No arbitrary blind signing, Solana program calls, token approvals, TON plugins or wallet destruction. Old signed whitelist canonical bytes remain unchanged; new payees use family-tagged canonical identities with exact chain scope.

Native SOL and TON are the explicit implemented scope. SPL tokens, Jettons, arbitrary calls, staking, NFT and TON-native mnemonic import are distinct future capabilities; unsupported requests fail explicitly. Existing EVM/Tron behavior is preserved. #26/#27 remain design-only. TON wallet fees are protocol-driven, so RPC fee estimates are not represented as an encoded on-chain max-fee guarantee.

## Acceptance
1. Public vectors/independent SDK fixtures reproduce Ed25519 paths, Solana addresses/signatures and TON V4R2 StateInit/address/message hashes; existing EVM/Tron vectors remain unchanged.
2. Strict decoders reject malformed/noncanonical/extra operations, recipient or amount tampering, wrong key/network context, unsupported modes and missing/wrong-family whitelist entries before a signature.
3. Temporary-home CLI create/unlock/address/balance/whitelist/transfer dry-run/send/lookup work against loopback mocks for each new family. No signed payload leaks in dry-run output. Broadcast errors remain uncertain where appropriate.
4. TUI chain selection, receive, native balance, immutable review, nickname confirmation and receipt presentation work for both families, with generation/key/chain pinning. Tron QR and full address are observed at 80×24 and 120×40.
5. Recovery/migration, legacy whitelist encoding, fee/nonce/freshness limitations and unsupported features are documented with source/test evidence.
6. fmt, strict all-target Clippy, full tests and final-head Linux/macOS CI pass. Update existing ai-insightquest draft PR #59 with Z coauthor on every new commit; no merge/release authority is inferred.

## Primary references
- https://github.com/satoshilabs/slips/blob/master/slip-0010.md
- https://solana.com/docs/core/transactions
- https://docs.ton.org/contracts/standard/wallets/mnemonics
- https://docs.ton.org/contracts/standard/wallets/v4
