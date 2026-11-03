# TON independent fixtures

`v4r2-sdk.json` was generated on 2026-11-02 by `generate.cjs` with pinned official packages
`@ton/ton@16.0.0`, `@ton/core@0.62.0`, `@ton/crypto@3.3.0` (see package versions inside the JSON).
The script uses the repository's public BIP39 test vector, Node's PBKDF2/HMAC and the official
SDK's V4R2 state initialization, address, internal-message serialization and signing. It writes
only public addresses, keys, signed fixture messages and hashes; no secret keys are printed.
The fixture is intentionally expired/future-dated for codec testing and is never broadcast.

Reproduce in a temporary directory: install the three pinned packages using `npm install
--ignore-scripts`, copy `generate.cjs` beside that node_modules, then run `node generate.cjs
/path/to/v4r2-sdk.json`. Do not run it against real wallet material.

Pinned runtime code `src/chains/ton/wallet-v4r2.boc` is the official MIT-licensed @ton/ton
WalletContractV4 bytecode. Code hash: `feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0`.
Source: https://github.com/ton-org/ton/blob/master/src/wallets/v4/WalletContractV4.ts
Contract: https://github.com/ton-blockchain/wallet-contract/blob/master/func/wallet-v4-code.fc

Copyright (c) Whales Corp. Permission is hereby granted, free of charge, to any person obtaining
 a copy of this software and associated documentation files (the "Software"), to deal in the
Software without restriction, including without limitation the rights to use, copy, modify,
merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons
to whom the Software is furnished to do so, subject to the following conditions: the above
copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT
HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
