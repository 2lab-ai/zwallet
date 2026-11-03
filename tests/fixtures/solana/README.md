# Native SOL independent vector

`native-transfer.json` was generated on 2026-11-02 using `@solana/web3.js@1.98.4`, `ed25519-hd-key@1.3.0`, and `bip39@3.1.0` against the repository's existing **public** `TEST_MNEMONIC` fixture. It contains public addresses and an intentionally synthetic transaction, no secret key or mnemonic. The recent blockhash is the base58 encoding of 32 bytes of value 9; this is not a funded or broadcast transaction.

The independent implementation derives indexes 0 and 1 at `m/44'/501'/{index}'/0'`, serializes a 1,000,000-lamport SystemProgram transfer, and signs it. Rust tests compare both addresses, the complete 150-byte signing message, the complete 215-byte signed transaction, and the base58 transaction signature.

Reproduce in a temporary directory with npm's lifecycle scripts disabled:

```sh
npm install --prefix /tmp/zwt-solana-vector --ignore-scripts --no-audit --no-fund @solana/web3.js@1.98.4 ed25519-hd-key@1.3.0 bip39@3.1.0
NODE_PATH=/tmp/zwt-solana-vector/node_modules node tests/fixtures/solana/generate.cjs
```

Protocol references:
- [Solana transaction structure](https://solana.com/docs/core/transactions/transaction-structure)
- [Solana SDK genesis constants](https://github.com/solana-labs/solana/blob/master/sdk/src/genesis_config.rs)
- [Solana fee structure](https://solana.com/docs/core/fees)
- [Solana sendTransaction](https://solana.com/docs/rpc/http/sendtransaction)

The current base fee of 5,000 lamports per signature is checked against the RPC quote. It is a protocol assumption, not a fee maximum encoded into the legacy message. The offline daemon cannot verify a recent blockhash's network/freshness. A genesis reply is an endpoint identity claim, not a consensus proof. Only native transfers are accepted; arbitrary programs, extra instructions, SPL tokens, durable nonces and versioned messages are rejected.
