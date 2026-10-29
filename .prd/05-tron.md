# zwallet — Tron (TRX and TRC20 USDT)

![WIP](https://img.shields.io/badge/status-WIP-yellow)

Tron addresses and TRX/TRC20 USDT balance reads are implemented. Signing, transfers and transaction lookup below remain planned. See [feature status](../docs/features.md).

Status: **in-progress**, 2026-10-29. Fixed request and scope: [`v0-3-tron/ssot.md`](v0-3-tron/ssot.md).
Units: [`v0-3-tron/units.md`](v0-3-tron/units.md). Driver: [`v0-3-tron/loop.md`](v0-3-tron/loop.md).
Every measurement below was taken on 2026-10-28 between 03:13Z and 03:31Z, read-only. Nothing
was signed or broadcast, and no faucet was used. Evidence IDs (E1…) are listed in §Evidence.

## Implementation boundary

Tron A implements address derivation, exact-case cache checks, verified receive addresses,
TRX and USDT reads, and the owner-permission warning. Signing, transfer policy, broadcasting
and transaction lookup remain planned until B1, B2 and C pass their gates. This document
retains those later contracts; it does not claim they are shipped.

## Problem

The owner asked on 2026-10-28: "같은 시드로 트론 월릿 주소도 생성하고 트론 밸런스 trc20 usdt까지 밸런스
볼수 있고 전송할수 있도록 기능 추가". zwallet has one chain family today, EVM
(`src/chains/mod.rs:27-29`, `enum Family { Evm(..) }`). G6 of [`01-spec.md`](01-spec.md) promises
that a new family fits behind the same boundary "without changing the wallet file or the daemon
protocol". Tron is that first test. It differs from EVM in four ways that matter for custody:

- **Addresses** are base58check strings (`T…`), case-sensitive. zwallet compares cached addresses
  with `eq_ignore_ascii_case` (`src/core/store.rs:713-715`, `src/commands.rs:371`).
- **Transactions** are protobuf, signed over `sha256(raw_data)`. There is no nonce. A signed
  transaction is valid until its own `expiration`.
- **Fees** are resources (bandwidth, energy). The only fee bound inside the signed bytes is
  `fee_limit`, and only on contract calls.
- **Accounts have permissions.** One contract type (`AccountPermissionUpdateContract`, 46) hands an
  account to other keys for good. The public test mnemonic's first Tron account already shows this
  (§Derivation).

## Goals

- **TG1 Address.** The same BIP39 seed gives each key a Tron address. Key 0's address is the one
  TronWeb derives by default from the same mnemonic; later keys follow zwallet's own index order,
  which other wallets may not share (Q2).
- **TG2 Balance.** TRX and USDT (TRC20) for the key, read-only, while locked or unlocked.
  `balance` without `--chain` includes Tron.
- **TG3 Transfer.** TRX and USDT to a whitelisted Tron payee, with `--dry-run` and `--wait`.
- **TG4 Same custody.** Same `wallet.json`, same sealed anchor, same whitelist signature format,
  same daemon protocol v2. The daemon decodes and judges Tron bytes itself, as it does EVM bytes.
- **TG5 TUI inputs.** Each Tron capability has its inputs, outputs, confirmations and errors
  written down for the TUI coverage task (§TUI).

## Non-goals (each with its reason)

| Not in this version | Why |
|---|---|
| Staking, freezing, delegating or renting energy/bandwidth (types 54–59, 11–13) | Each is another contract type the daemon would have to judge. Fees are paid by burning TRX; the owner asked for balance and send. |
| TRC10 tokens (type 2, `TransferAssetContract`) | The owner named TRC20 USDT. No issuer publishes a TRC10 stablecoin. The test account TWer2… holds 7 TRC10 assets nobody asked for (M7), so listing them would show names an attacker picked. |
| Multisig, permission changes (type 46, `AccountPermissionUpdateContract`) | It hands the account to other keys permanently (§Daemon). Never signed. |
| `call` / `invoke` on Tron, contract entries on `tron` | A contract entry delegates authority (04 §What this guarantees). Nothing asked for it. `require_contract_calls` already returns `CAPABILITY_UNSUPPORTED` (`src/chains/mod.rs:1482-1498`). |
| Memos (`raw_data.data`, field 10) | Each memo costs 1 TRX (`getMemoFee` = 1,000,000 sun, M5). Tron exchanges give each user an address. The daemon refuses the field. |
| `sign-message` on Tron (TIP-191) | Not asked for. A message signature can be spending authority, which is why EVM gates it. Tron returns `CAPABILITY_UNSUPPORTED`. |
| USDC and other TRC20 tokens | Circle stopped minting USDC on TRON on 2024-02-21 and lists no TRON contract (E9). The registry takes issuer-direct deployments only (04 §Registry tokens). |
| Nile or Shasta testnet | No issuer-direct USDT there, and this work requests no faucet. The send path is tested against a local mock node; one mainnet send is owner-gated (TR23). |
| An API key header (`TRON-PRO-API-KEY`) | [`01-spec.md`](01-spec.md) lists "any third-party API key" as a non-goal. The default node needs none (M1). |
| Shielded transfers (type 51) | Not asked for; a different signature scheme. |

## Design

### Family and registry row

- `Family::Tron(tron::TronChain { default_rpc, explorer, genesis })`. Chain key `tron`, name
  `Tron`, symbol `TRX`, decimals 6. It goes after `avalanche` in `CHAINS`; `balance` already sorts
  testnets last.
- **chainId = 728126428** (`0x2b6653dc`). It is what Tron's own JSON-RPC answers to `eth_chainId`:
  the last 4 bytes of the genesis block id (`TronJsonRpcImpl.java:426-427`, E6; M6). zwallet uses
  it for `chains` output and for file names (`nonce-<chainId>-…lock`, `sends-<chainId>-…jsonl`,
  `src/chains/mod.rs:651-656`). It never goes into a Tron transaction; Tron has no chain id there.
- In those file names the Tron account is its 21-byte hex (`41…`, lower case), not the base58
  string. `SendLock::acquire` and `journal_path` lower-case the address
  (`src/chains/mod.rs:651-656`, `:856-862`), which would merge base58 strings that differ only in
  case.
- Code lives in `src/chains/tron/{mod,address,tx,policy,registry,rpc}.rs`. Two things move out of
  `evm` because they now span families:
  - `ListView`, `NO_WHITELIST`, `MESSAGE_SIGNING_COMMAND`, `VERIFY_COMMAND`
    (`src/chains/evm/policy.rs:29-54`) → `src/chains/policy.rs`, unchanged.
    `FamilyKeys::sign_tx` keeps its signature (`src/chains/mod.rs:248-258`).
  - `WHITELIST_RULES` and `DEFAULT_FEE_CAPS_WEI` (`src/chains/evm/registry.rs:123-146`) →
    `src/chains/mod.rs`, so `chain_known` covers `tron` and the defaults include its cap.
- `src/core` stays chain-agnostic. `src/daemon` changes only its imports.

### Derivation — decided: `m/44'/195'/0'/0/i`

- **Path.** Key index `i` maps to `m/44'/195'/0'/0/i`. SLIP-44 assigns coin type 195 to TRX
  (`slip-0044.md:228`, E1). TronWeb, TRON's official SDK, uses `m/44'/195'/0'/0/0` as its default
  and refuses any path outside `m/44'/195'` (`src/utils/constants.ts:6-7`,
  `src/utils/accounts.ts:40-50`, E2). Ledger's Tron app allows only `44'/195'` (`Makefile:43`,
  E3). The layout is the one zwallet's EVM path uses (`m/44'/60'/0'/0/i`,
  `src/chains/evm/mod.rs:48-50`): the last level counts keys. The claim is key 0 only: its path is
  TronWeb's default, so a wallet that derives TronWeb's default path shows key 0's address. Which
  level other wallets count for their second account is not checked ([unverified], Q2).
- **Address.** `0x41 ‖ keccak256(uncompressed public key without the 0x04 byte)[12..32]`, then
  base58check (4-byte checksum = first 4 bytes of `sha256(sha256(payload))`). 34 characters,
  first character `T`.
- **Test vector**, mnemonic `test test test test test test test test test test test junk` (the
  A1 mnemonic). Computed twice, by two implementations that share no code: TronWeb 6.5.0
  (`generateAccountWithMnemonic`, ethers HD node) and `@scure/bip32` 1.7.0 + `@scure/bip39`
  1.6.0 + `@noble/curves` 1.9.7 + `@scure/base` 1.2.6. Both printed the same (E4):

  | i | path | address | 21-byte hex |
  |---|---|---|---|
  | 0 | `m/44'/195'/0'/0/0` | `TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6` | `41e2e1a54926527fbb4e4420de4c6bab82beaee24d` |
  | 1 | `m/44'/195'/0'/0/1` | `TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C` | `419705bf55c3dcc6d277ebb8fe2a68762268822ba2` |

  The same scure script printed `0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266` for
  `m/44'/60'/0'/0/0`, which is A1, so the script itself is sound. Both converted the USDT contract
  `TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t` ↔ `41a614f803b6fd780986a42c78ec9c7f77e6ded13c`.
- **The test mnemonic is public, and its Tron accounts are live.** On 2026-10-28 `getaccount` for
  TWer2… returned balance 8,000,010 sun and an `owner_permission` whose only key is
  `TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk`, not TWer2… itself (M7). Someone who had the mnemonic gave
  the account to another key. Funds sent there belong to that key. This is the drain that type 46
  enables (§Daemon). TRON-2 records that `getaccount` answer as a mock fixture for the warning;
  the live test (TR6) pins neither the balance nor the key, which can change.
- **Implementation.** The existing `MnemonicBuilder` path (`src/chains/evm/mod.rs:52-65`, alloy's
  `coins-bip32`) with the Tron path; `alloy::primitives::keccak256`; `bs58` 0.5.1 with feature
  `check`; `sha2` 0.10.9. `bs58` 0.5.1 and `sha2` 0.10.9 are already in `Cargo.lock` through
  `coins-bip32` 0.12.0 and `coins-core` 0.12.0, so no new crate enters the build. The index limit
  stays `KEY_INDEX_LIMIT` (2^31).

### Network access — decided: PublicNode by default, TronGrid documented

Measured 2026-10-28 03:15Z, each call `POST` with a JSON body, no API key:

| Endpoint | `/wallet/getblock` `{"detail":false}` (3 runs) | 60 requests, 15 at a time (03:16:47Z) | Other calls below |
|---|---|---|---|
| `https://tron-rpc.publicnode.com` | 200, 0.44 s, 542 B | 60 × 200 | all 200, 0.43–0.47 s |
| `https://api.trongrid.io` | 200, 0.33–0.58 s, 542 B | 5 × 200, 52 × 429, 3 × 403 | all 200, 0.31–0.33 s |
| `https://tron.drpc.org` | 400 `"method is not available on free plan"` | — | — |
| `https://rpc.ankr.com/http/tron` | 403 `"API key is not allowed to access blockchain"` | — | — |

TronGrid's 429 body (03:17Z): `"request rate exceeded the allowed_rps(3), and the query server is
suspended for 5 s"`. Ten sequential requests right after that were all 200. TronGrid's own page
says requests without a key face "strict rate limiting or rejection" and gives no number (E10).

Decisions:
- **Default RPC `https://tron-rpc.publicnode.com`.** The EVM defaults are PublicNode too
  ([`02-architecture.md`](02-architecture.md) §CLI contract). TronGrid without a key allows
  3 requests/s; `balance` without `--chain` reads every network at once.
- **Override:** `--rpc <url>`, then `ZWALLET_RPC_TRON`, then the default (`Chain::resolve_rpc`,
  `src/chains/mod.rs:179-188`). URL redaction is unchanged (`redact_url`).
- **Chain identity.** Before the first read, `POST /wallet/getblockbynum {"num":0}` must return
  blockID `00000000000000001ebf88508a03865c71d452e25f4d51194196a1d22b6653dc` (both endpoints, M6;
  Nile answers `0000000000000000d698d4192c56cb6be724a558448e2684802de4d6cd8690dc`). Otherwise
  `RPC_ERROR` naming the mismatch. This is the Tron form of the `eth_chainId` check.
- **HTTP API, not JSON-RPC.** Tron's JSON-RPC answers `eth_sendRawTransaction` with "does not
  exist/is not available" (`TronJsonRpcImpl.java:1369-1371`, E6); the HTTP API `/wallet/*` does
  everything below. Addresses go as `"visible": true` base58 in reads.

The calls, all measured 200 on both endpoints except where noted:

| Need | Call | What zwallet reads |
|---|---|---|
| chain identity | `/wallet/getblockbynum {"num":0}` | `blockID` |
| TRX balance, activation, permissions | `/wallet/getaccount {"address":T…,"visible":true}` | `balance` (sun); `{}` = account not activated (M8); `owner_permission` |
| TRC20 balance | `/wallet/triggerconstantcontract` `balanceOf(address)` | `constant_result[0]` (uint256); `result.result` |
| reference block | `/wallet/getblock {"detail":false}` | `blockID`, `block_header.raw_data.number`, `.timestamp` (542 B; `getnowblock` is 372 KB) |
| resources | `/wallet/getaccountresource` | `freeNetLimit`, `freeNetUsed`, `NetLimit`, `NetUsed`, `EnergyLimit`, `EnergyUsed` (absent = 0) |
| prices | `/wallet/getchainparameters` | `getTransactionFee`, `getEnergyFee`, `getCreateAccountFee`, `getCreateNewAccountFeeInSystemContract` |
| energy estimate | `/wallet/triggerconstantcontract` `transfer(address,uint256)` from the key | `energy_used` (includes `energy_penalty`), `result.result` |
| broadcast | `/wallet/broadcasthex {"transaction": <hex of the signed Transaction>}` | `result`, `code`, `txid`, `message` (BroadcastHexServlet.java:23-43, E6) |
| confirmation | `/wallet/gettransactioninfobyid`, `/wallet/gettransactionbyid` | `blockNumber`, `receipt.result`, `receipt.*`, `fee`; `ret[0].contractRet` |
| pending | `/wallet/gettransactionfrompending` | the pending transaction or `{}` (FullNodeHttpApiService.java:495) |

`/wallet/estimateenergy` answered `"this node does not support estimate energy"` on both endpoints
(M3), so the estimate comes from `triggerconstantcontract`. `broadcasthex` was not called with a
payload. A payload-free `GET` returned 405 there and 404 for an unknown path, on both endpoints, so
the servlet is mounted (M9). Whether a `POST` needs a key is open (Q3); PR C closes it before it
starts (§Open questions).

### The registry — decided: USDT only

| Token | Contract | Decimals | Source |
|---|---|---|---|
| USDT (Tether) | `TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t` | 6 | Tether's supported-protocols page lists Tron with this address, "The integration path is the same as any TRC20 compatible token" (E8). `decimals()` = 6 and `symbol()` = `USDT` on both endpoints (M2). |

- Tether issues it directly, so it fits the issuer-direct rule (04 §Registry tokens).
- **USDC: no.** Circle, 2024-02-21: "Effective immediately we will no longer mint USDC on TRON";
  Circle Mint transfers were supported "through February 2025". Circle's USDC address page lists
  38 chains and no TRON (both fetched 2026-10-28, E9).
- **Nothing else.** No other issuer-direct stablecoin was asked for.
- **USDT's `transfer` returns false on success.** The mainnet USDT transfer `21b776c7…a690`
  (block 87494658) has `receipt.result: "SUCCESS"` and `contractResult` = 32 zero bytes (M4). So
  zwallet judges a TRC20 transfer by the receipt's `result`, never by return data.
- A live test reads `symbol()` and `decimals()` of every Tron registry row, as
  `every_registry_token_answers_its_symbol_and_decimals` does for EVM (`tests/live.rs:452`).

### The transaction the CLI builds and the daemon signs

`raw_data` (`Transaction.raw`, `core/Tron.proto:431-445`, E5). zwallet writes exactly these
fields, in this order:

| Field | # | Wire | zwallet's value |
|---|---|---|---|
| `ref_block_bytes` | 1 | bytes(2) | bytes 6–7 of the reference block number, big-endian |
| `ref_block_hash` | 4 | bytes(8) | bytes 8–15 of the reference block id (TransactionCapsule.java:519-526) |
| `expiration` | 8 | varint | `timestamp + 60,000` ms |
| `contract` | 11 | message, exactly one | `{type: 1 or 31, parameter: Any{type_url, value}}`; `provider` (3), `ContractName` (4), `Permission_id` (5) never written (`Tron.proto:380-384`) |
| `timestamp` | 14 | varint | the reference block's timestamp (ms) |
| `fee_limit` | 18 | varint | TriggerSmartContract only (§Fees) |

Never written: `ref_block_num` (3), `auths` (9), `data` (10), `scripts` (12).

- **TRX:** type 1, `type_url` `type.googleapis.com/protocol.TransferContract`, value
  `TransferContract {owner_address: 1 = 0x41‖owner, to_address: 2 = 0x41‖to, amount: 3 = sun > 0}`
  (`balance_contract.proto:32-36`).
- **TRC20:** type 31, `type_url` `type.googleapis.com/protocol.TriggerSmartContract`, value
  `{owner_address: 1, contract_address: 2 = the registry token, data: 4}` with `data` =
  `a9059cbb ‖ 12 zero bytes ‖ to (20 bytes, no 0x41) ‖ amount (uint256)`, 68 bytes;
  `call_value` (3), `call_token_value` (5), `token_id` (6) never written
  (`smart_contract.proto:74-81`).
- **txID = sha256(raw_data bytes)** (`TransactionCapsule.java:584-587` `getRawHash`, `:721-725`
  `getTransactionId`). The signature is not part of it, so the hash is final before the send and
  cannot change after it.
- **Signature:** secp256k1 over the 32-byte txID, no prefix; 65 bytes `r ‖ s ‖ v`. java-tron's own
  signer writes `v` = recovery id 0/1 (`ECKey.java:978-986`) and its reader adds 27 when `v < 27`
  (`Rsv.java:17-24`). zwallet writes 0/1 and low-s (k256 normalizes). Both forms are on mainnet:
  `21b776c7…` has `v = 0x1b`, `e4e8962a…` has `v = 0x00` and a high `s`, and both recover to their
  `owner_address` (M4). The node does not require low `s` (`ECKey.java:923-941`).
- **Signed form** for broadcast: `Transaction {raw_data: 1, signature: 2}`.
- **Bandwidth bytes** = size of that Transaction + 64 per contract
  (`BandwidthProcessor.java:116-128`). Measured (M4): TRX 267 bytes for a 133-byte `raw_data`,
  USDT 345 for 211; 1+2+133+1+1+65+64 = 267 and 1+2+211+1+1+65+64 = 345.

**Golden vectors** (node-built, unsigned, 2026-10-28 03:15Z, M10). A ~20-line Python encoder of the
table above reproduced both byte-for-byte, and `sha256` equals each `txID`. They pin the codec
only: the node set `timestamp` to its own clock and `expiration` 58,474 ms later, not
zwallet's "reference block timestamp + 60,000 ms". TRON-6 tests that rule on its own:

```text
TransferContract  TYUqBpE3Xxo3UWQAKVvwTfzYMdTm57x1C2 -> TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C, 1 sun
raw_data_hex 0a0210082208a7a95ebd17ea3b6f40c08e898498345a65080112610a2d747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e5472616e73666572436f6e747261637412300a1541f6ed094625cafe392ef7aa49cfa5691ede69b6561215419705bf55c3dcc6d277ebb8fe2a68762268822ba2180170d6c585849834
txID         c3d1f7cb446dc2b4789996f17af93ffa356ac394ca0baad522c9e9db885e9f42
TriggerSmartContract  TPdMMFcLKgJiUSmYsViAFvf6Nsb2WV52vs: USDT transfer(TPjjvM…, 1000000), fee_limit 30000000
raw_data_hex 0a0210082208a7a95ebd17ea3b6f40c08e898498345aae01081f12a9010a31747970652e676f6f676c65617069732e636f6d2f70726f746f636f6c2e54726967676572536d617274436f6e747261637412740a154195d06912584d95ac7c6dee580ee23154b59b784e121541a614f803b6fd780986a42c78ec9c7f77e6ded13c2244a9059cbb0000000000000000000000009705bf55c3dcc6d277ebb8fe2a68762268822ba200000000000000000000000000000000000000000000000000000000000f424070e4c88584983490018087a70e
txID         f0cde1f65566e05c12b0c4f97a34f7fe0f83e7950ea4c6d079d26d834a27505d
```

Signed mainnet vectors for signer recovery (block 87494658, M4): `21b776c7957ba1213b8616414912c9e81643e15e3e55b59fe143b991fa35a690`
(owner `TPdMMFcLKgJiUSmYsViAFvf6Nsb2WV52vs`, `v = 0x1b`) and
`e4e8962a61011cfff562c4443c0d7d36bdea09aea3674b7176d31528ffc2f801` (owner
`TYUqBpE3Xxo3UWQAKVvwTfzYMdTm57x1C2`, `v = 0x00`, high `s`); TRON-4 copies their `raw_data_hex`
and `signature` into a fixture.

**Decided: hand-written protobuf, no crate.** Reasons:
1. The daemon must refuse what it does not understand. `prost` (0.14.4, tokio-rs, 2026-06-07)
   skips unknown fields while decoding (`prost-derive/src/lib.rs:191`, `skip_field`, E7), so a
   decoded view could miss a field the node acts on. A hand-written decoder refuses any field not
   in the table.
2. Two message types, ten fields. Encoder and decoder are about 150 lines; the Python replica
   of the encoder was 20.
3. Nothing new enters the key-holding process. `prost` would add `prost-derive` and `bytes`.
4. The decoder re-encodes what it decoded and requires the same bytes, as the EVM decoder does
   (`src/chains/evm/tx.rs:408`). A second encoding of the same values never passes.

### The daemon's judgement (the security core)

`Request::SignTx {chain: "tron", index, tx}` (`src/core/protocol.rs:90-94`) carries only
`tx` = the 0x-hex `raw_data` bytes. The daemon never receives a txID, an amount, a recipient or a
fee from the CLI. `FamilyKeys::sign_tx` for Tron, in order:

1. **Decode whole** (`tron::tx::decode_raw`), else `VALIDATION_ERROR` "refusing to sign: …". The
   same strictness holds at every level: `raw_data`, the `Contract` inside it, the `Any` inside
   that (`type_url` = 1, `value` = 2, nothing else) and the `TransferContract` or
   `TriggerSmartContract` inside that. At each level a field outside the table, a field twice, a
   field out of order, a wrong wire type, a non-minimal varint or a length past the end refuses.
   - at most 1 KiB; `raw_data` fields only from {1, 4, 8, 11, 14, 18}, lengths exact (field 1:
     2 bytes, field 4: 8 bytes, addresses 21 bytes starting `0x41`); no trailing bytes;
   - exactly one contract; type 1 or 31 only. Any other type is named in the refusal; type 46
     says "it hands this account to other keys";
   - `type_url` exactly the one for that type; no `provider`, `ContractName`, `Permission_id`
     (absent = 0 = owner; the node picks the permission by this id,
     `TransactionCapsule.java:468-492`);
   - TransferContract: fields 1–3 only; `amount` > 0 and below 2^63; no `fee_limit` in
     `raw_data`;
   - TriggerSmartContract: fields 1, 2 and 4 only, so no `call_value` (its field 3),
     `call_token_value` (5) or `token_id` (6); `data` present;
   - re-encoding the decoded fields gives the input bytes.
2. **Owner = this key.** `owner_address` must equal `0x41 ‖` this key's 20-byte address, else
   `VALIDATION_ERROR`.
3. **Expiration bounded.** `timestamp < expiration ≤ timestamp + 300 s` and `expiration ≤ the
   daemon's clock + 300 s`, else `VALIDATION_ERROR` naming the clock. The node accepts up to 24 h
   (`Manager.java:853`, `Constant.java:19`), and anyone who holds signed bytes can broadcast them
   until then. The CLI writes 60 s. An already expired transaction is not refused here; the node
   refuses it and nothing can be lost.
4. **The list state** (`ListView`, unchanged): `Unusable` → `WHITELIST_INVALID`, everything.
   `NoList` → `WHITELIST_INVALID`, everything, with the first `whitelist add` as `command` when an
   entry could allow the action (04 step 2). EVM's one exception, a native self-transfer, does
   not exist on Tron: the node refuses TRX to oneself (`TransferActuator.java:111-113`), and the
   daemon refuses it too.
5. **The action** (`tron::policy::check_tx`), `WHITELIST_DENIED` unless:

   | Decoded | Signed only if |
   |---|---|
   | `native-transfer` (type 1) to the owner itself | never: `WHITELIST_DENIED`, no `command` ("the node refuses TRX sent to its own sender") |
   | `native-transfer` (type 1) | `to` is an address entry whose chains name `tron` (`*` does not count) |
   | `transfer-call` (type 31, `data` = canonical `transfer(address,uint256)`) | the contract is a Tron registry token; the recipient is an address entry naming `tron`; the recipient is not a Tron registry token |
   | any other selector, or `data` that is not exactly the 68-byte canonical encoding (`approve` `0x095ea7b3`, `transferFrom` `0x23b872dd`, anything else) | never; no `command` |
   | a TRC20 `transfer` of a token outside the registry | never; no `command` |

   An unlisted payee gets `command: zwallet whitelist add <nick> T… --chains tron`. An entry that
   exists but does not name `tron` gets no `command` (04 §What a refusal carries).
6. **The fee** (after the action, as 04 step 4): type 31 needs `fee_limit` ≤ the verified list's
   `maxFeeWei.tron` (in sun), else `FEE_CAP_EXCEEDED` with `command: zwallet whitelist fee-cap tron
   <fee_limit in TRX>`, `feeWei` and `capWei` (field names kept; the values are sun). Type 1
   carries no fee field; it still needs a readable `tron` cap, as every EVM signature needs one
   (04 §Policy). A list with no `tron` cap refuses either type with `FEE_CAP_EXCEEDED` and
   `command: zwallet whitelist fee-cap tron <max(fee_limit, 30)>` in TRX: for type 1 that is the
   built-in default, 30, because `fee-cap` refuses 0 (04 §As built, part C item 5).
7. **Sign** `sha256(raw_data)` and return `{rawData, signature, txHash, decoded}` (hex; `txHash` =
   txID). The daemon computed the hash from bytes it decoded whole; it never signs a hash it was
   given.

**Mapping onto [`04-whitelist.md`](04-whitelist.md) §What this guarantees, line by line:**

| 04 says | On Tron | |
|---|---|---|
| Native coin and registry tokens leave only to address entries, or as a native transfer to the key itself | TRX and USDT leave only to address entries naming `tron`. No self-transfer: the node refuses it | narrowed |
| ERC-20 `transfer`/`approve`/`transferFrom` judged whatever the entries say; approvals revoke-only | only a registry token's `transfer`; no `approve` of any amount, so no revoke either (an approval made by another wallet from this seed is revoked there) | narrowed |
| Other calls only to contract entries with listed selectors | none: no contract entries on `tron`, no `call`/`invoke` | narrowed |
| Fee cap on every signature | `fee_limit` ≤ cap on type 31; type 1 has no fee field. Bandwidth and activation fees are outside every signature (below) | new limit |
| A list that cannot be trusted refuses every signature | same | same |
| No whitelist yet: native self-transfer only, under the built-in cap | nothing is signed | narrowed |
| `sign-message` gated | no message signing on Tron | narrowed |
| Never a raw hash, typed data or type 4 | never a hash the daemon did not compute from bytes it decoded whole; never type 46 or any type other than 1 and 31 | new |
| — | owner must be this key; `Permission_id` absent; expiration within 5 min; no memo | new |

Not guaranteed, added to 04's list: **bandwidth and activation fees** (protocol-set, §Fees);
**which Tron network** a reference block belongs to (the daemon cannot check block hashes; bytes
signed against a Nile block are valid only on Nile, and the CLI checks the genesis block first);
**that the owner permission is still this key** (only the CLI can read it; §CLI).

### The whitelist

- **Entry, stored:** `{nickname, kind: "address", address: "0x" + the 20-byte body in lower-case
  hex, chains: ["tron"]}`. That is exactly today's shape: `check_format` and `check_shapes` pass
  unchanged (`src/core/whitelist.rs:326-426`, `:689-720`). The canonical bytes, `SIGNING_DOMAIN`,
  `LIST_VERSION` 1, the digest, the anchor and `SEALED_VERSION` do not change. The existing
  golden-bytes test passes untouched, which proves it.
- **Entry, shown:** the base58check `T…` of `0x41 ‖ body`, as EVM entries are stored lower-case and
  shown EIP-55. The change summary before the password shows both: `T…` and "(stored as 0x…, the
  same 20 bytes)". `whitelist list --json` gives every entry `family: "evm" | "tron"` and the
  address in its family's form, so no agent reads a Tron body as an EVM address.
- **Added with** `zwallet whitelist add <nick> <T…> [--chains tron]`. A `T…` address defaults to
  `--chains tron`. Checked before the password (`add_change`, `src/commands/whitelist.rs:151-175`),
  each `VALIDATION_ERROR`:
  - a `T…` address with a chain other than `tron`, or with `*`;
  - a `0x` address with `--chains tron` ("a Tron payee is given as its T… address");
  - `tron` together with any other chain (one 20-byte body would name two different keys'
    addresses);
  - a bad checksum, a length other than 34, a first byte other than `0x41`;
  - `--contract` with `tron`.
- **`validate` gains two rules**, so a list that breaks them is never signed, whoever wrote it
  (today it checks only that each chain is known and that no contract entry sits at a registry
  token, `src/core/whitelist.rs:294-318`): an entry naming `tron` names no other chain, and an
  entry naming `tron` is an address entry. `ChainRules` gains `family_of(chain) -> &str`, so core
  stays chain-agnostic. `check_shapes` (the read side) is unchanged.
- **`*` means every EVM chain.** Lists signed before this version said `*` when "every chain" was
  the EVM set. If `*` covered `tron`, a signed list would start paying the Tron address that shares
  a body with an EVM payee, which for an exchange deposit address or a Safe is nobody's key.
  `Whitelist::payee` gains a `wildcard: bool`; the EVM policy passes `true`, the Tron policy
  `false`. README's "`'*'` for every chain" becomes "every EVM chain", and the release note of the
  version that adds the Tron signer says: "`*` in a whitelist now means every EVM chain; it never
  covers `tron`. A Tron payee is added with `whitelist add <nick> T… --chains tron`".
- **One known collision.** "Each address appears under one nickname" compares stored bodies, so an
  EVM entry and a Tron entry with the same 20 bytes cannot both exist. That happens only when one
  private key is used on both chains. Accepted: the second `add` is refused with a message, nothing
  is lost.
- **Existing lists stay valid.** Their bytes and signature are untouched and verify as today. The
  only semantic change narrows (`*`). They have no `tron` fee cap, so a first Tron send needs two
  password edits, each named by the refusal: `whitelist add <nick> T… --chains tron`, then
  `whitelist fee-cap tron 30`. New lists (first `add`, `reset`) get the default cap.
- **Older zwallet binaries** reading a list with a `tron` entry or a `tron` fee cap: `check_shapes`
  checks a chain key's shape, not membership (`src/core/whitelist.rs:681-688`), and `verify`
  checks the format only, never `validate`'s chain rules (`src/core/whitelist.rs:763-768`), so
  the list still verifies and their daemon signs EVM as before. Their edits refuse it ("unknown
  chain", `validate`, `src/core/whitelist.rs:294-324`, called only by edits, `:1437`). That
  happens after the first Tron entry, and also after any first `add` or `reset` on a build with
  the Tron signer (PR B2, which adds the `tron` default cap), since the new list's default caps
  then include `tron`. From then on, edit with a Tron-aware
  zwallet.
- **`@nick`** (`transfer --chain tron --to @bob`) resolves a `tron` entry to its `T…`; an entry
  that does not name `tron` is `VALIDATION_ERROR` naming its chains.

### Fees

Tron charges two resources (M5, `getchainparameters`, 03:15Z):

| Parameter | Value | Meaning |
|---|---|---|
| `getTransactionFee` | 1,000 sun/byte | bandwidth burned when neither staked nor free bandwidth covers the whole transaction |
| `getFreeNetLimit` | 600 bytes/day | free bandwidth per activated account |
| `getEnergyFee` | 100 sun/energy | energy burned beyond staked energy, up to `fee_limit` |
| `getCreateNewAccountFeeInSystemContract` | 1,000,000 sun | added to the fee when TRX goes to an account that does not exist (`TransferActuator.java:45-57` execute, `:129-131` validate) |
| `getCreateAccountFee` | 100,000 sun | the bandwidth bill of such a transfer when staked bandwidth does not cover it; it replaces the normal bandwidth bill (below) |
| `getCreateNewAccountBandwidthRate` | 1 | staked bandwidth such a transfer uses: bytes × this rate |
| `getMaxFeeLimit` | 15,000,000,000 sun | the chain's ceiling on `fee_limit` |

Bandwidth comes from staked bandwidth if it covers the whole transaction, else free bandwidth if it
does, else bytes × `getTransactionFee` is burned (`BandwidthProcessor.java:160-170`). **A TRX
transfer to an account that does not exist at execution is billed on another branch instead**:
`contractCreateNewAccount` is true (`BandwidthProcessor.java:259-287`), so the processor calls
`consumeForCreateNewAccount` and skips the normal bill (`continue`, `:139-152`). That branch takes
bytes × `getCreateNewAccountBandwidthRate` of staked bandwidth if it is there, else burns
`getCreateAccountFee`; free bandwidth is never used (`:192-205`, `:248-257`). The two bills are
alternatives, never summed. Which branch runs is decided when the block executes, by whether the
payee exists then.

Measured costs:
- TRX transfer: 267 bytes → 0 TRX with free bandwidth left, else 0.267 TRX. To an account that
  does not exist: 1 TRX (system-contract fee) plus 0.1 TRX in place of the 0.267 (or staked
  bandwidth in place of both bandwidth bills), at the 2026-10-28 prices.
- USDT transfer (simulated from `TPdMMF…`, 1 USDT): to a holder with a balance, 64,285 energy
  (penalty 49,635) = 6.4285 TRX if burned; to an address holding no USDT, 130,285 energy (penalty
  100,635) = 13.0285 TRX (M3). The mainnet transfer `21b776c7…` used exactly 130,285 energy and 345
  bytes, all from staked resources (M4). Plus 0.345 TRX bandwidth without free bandwidth.

Decisions:
- **The cap bounds `fee_limit`**, the only fee bound in the signed bytes. Bandwidth and activation
  fees are protocol-set and outside every signature, as the OP-stack L1 fee is on EVM. They are
  bounded by the CLI's preflight and shown; the daemon cannot see them.
- **Default `maxFeeWei.tron` = 30,000,000 sun (30 TRX).** That is 2.3 × the measured worst case
  (13.0285 TRX), room for dynamic energy (`getAllowDynamicEnergy` = 1). Changed with
  `whitelist fee-cap tron <TRX>`.
- **The CLI sets `fee_limit` = ceil(`energy_used` × `getEnergyFee` × 1.2)**, at least
  1,000,000 sun. `energy_used` comes from simulating this very transfer
  (`triggerconstantcontract`); a simulation whose `result.result` is not `true` is `RPC_ERROR`
  before signing, naming the revert.
- **Preflight, before signing**, from `getaccount`, `getaccountresource`, `getchainparameters` and,
  for TRX, `getaccount(to)`. It computes two totals in sun. Every price in them is the value
  `getchainparameters` returned in this preflight, never a constant from M5: `getTransactionFee` (P_tx), `getEnergyFee` (P_e),
  `getCreateNewAccountFeeInSystemContract` (P_sys), `getCreateAccountFee` (P_ca),
  `getCreateNewAccountBandwidthRate` (R). `bytes` is the signed transaction's bandwidth size
  (§The transaction).
  - **`estimatedCostSun`**, what the send is expected to burn, on the branch today's state
    selects:
    - TRX to a payee that exists: amount + bandwidth, where bandwidth = 0 if staked or free
      bandwidth covers `bytes`, else `bytes` × P_tx;
    - TRX to a payee that does not exist: amount + P_sys + (0 if staked bandwidth covers
      `bytes` × R, else P_ca); no normal bandwidth bill (the branch replaces it);
    - USDT: bandwidth as for an existing payee + max(0, `energy_used` − (`EnergyLimit` −
      `EnergyUsed`)) × P_e.
  - **`maxCostSun`**, the most the signed transaction can burn at these prices, with no staked or
    free bandwidth assumed (another transaction can use them first), on whichever branch runs:
    - TRX to a payee that exists at the preflight as a normal account: amount + `bytes` × P_tx.
      java-tron deletes an account only when a contract self-destructs (`deleteContract`,
      `RepositoryImpl.java:488-491`, `TransactionTrace.java:373-378`), so for a normal account
      the new-account branch cannot run;
    - TRX to a payee that does not exist at the preflight, or exists as a contract account
      (`getaccount` `type: "Contract"`, which can self-destruct before the block): amount +
      max(P_sys + P_ca, `bytes` × P_tx). The first term is the new-account branch with staked
      bandwidth gone; the second is the normal branch. Each is the exact bill of its branch and
      only one runs, so this is the exact maximum, not a sum of both;
    - USDT: `bytes` × P_tx + the signed `fee_limit` (the node burns at most `fee_limit` for
      energy, whatever energy the call really uses, `VMActuator.java:583-621` and
      `ReceiptCapsule.java:275-302`, §Open questions; it is 1.2 × the simulated cost or more).
  - **What no field bounds:** a change of `getTransactionFee`, `getEnergyFee` (it changes how much
    energy `fee_limit` buys, not the TRX it caps), `getCreateAccountFee` or
    `getCreateNewAccountFeeInSystemContract` by a governance proposal between the preflight and
    the block. `maxCostSun` is at the preflight's prices, and `fees.pricesAtBlock` names the
    block they were read at.
- **The balance check uses `maxCostSun`.** `INSUFFICIENT_FUNDS` before signing when the TRX balance
  is below `maxCostSun` (`shortfall` = `maxCostSun` − balance), even when it covers
  `estimatedCostSun`; when the USDT balance is below the amount; or when the sender's account is
  not activated (`getaccount` = `{}`: "this Tron address holds no TRX yet; send it TRX first, for
  bandwidth and energy"). This is stricter than the node, on purpose: an account that would pay
  only with staked energy and bandwidth, and holds less liquid TRX than `fee_limit` + `bytes` ×
  P_tx, is refused, though the node might accept the transaction. A send never depends on
  resources another transaction can take first.
- **Shown before sending**: dry-run and live output carry `fees: {bandwidthBytes, bandwidthSource:
  "staked"|"free"|"burn", bandwidthSun, activationSun, energy, energySun, feeLimit,
  estimatedCostSun, maxCostSun, estimatedCost, maxCost, pricesAtBlock}` (`estimatedCost` and
  `maxCost` in TRX). "TRX left" anywhere (`nativeBalance` − cost, the TUI review) is balance −
  `maxCostSun`, labelled "at least, at the prices of block N" (`pricesAtBlock`).
- **After the send**, `--wait` adds `feeSun`, what the receipt says was burned. It can exceed
  `estimatedCostSun` (energy above the simulation) and stays within `maxCostSun` at unchanged
  prices. A call that runs out of energy is `success: false` (`OUT_OF_ENERGY`) with up to
  `fee_limit` burned, and the output says so.

### Send safety without a nonce

[`02-architecture.md`](02-architecture.md) §Send safety is built on the EVM nonce. Tron has none,
so:
- **Kept:** one send per key and chain (`SendLock`, `src/chains/mod.rs:644-679`); the send journal,
  appended and fsynced before the broadcast; the 10-minute duplicate guard on chain, kind, `to` and
  base-unit amount, with `--allow-duplicate`.
- **Refused on `tron`** with `VALIDATION_ERROR` before any RPC: `--nonce`, `--replace`, `--bump`,
  `--allow-pending`, `--max-fee-gwei` ("Tron has no nonce; …").
- **Journal line** `{ts, chain, from, txHash, to, value, amountRaw, kind, token?, expiresAt}`: no
  `nonce`; `txHash` is the txID as Tron writes it, 64 lower-case hex digits without `0x`; `to` is
  `T…`. The journal's shape checks (`is_address`, `entry_hash`, `src/chains/mod.rs:708-724`) get a
  Tron arm, so these lines are not shown as `<invalid>`.
- **Expiration is an observation, never proof that nothing was paid.** A transaction past its
  `expiration` cannot enter a later block. But a node that has no record of it may lag or not
  index it, so its absence does not show the transaction was never included. zwallet therefore:
  - takes time only from the node it asks: that node's head block (`getblock`, read in the same
    lookup), never this machine's clock;
  - reports `status: "expired-unseen"` only when all of these hold: the expiration is known (the
    txID is in this key's send journal, which records `expiresAt`); that node's head block
    timestamp is past `expiresAt` + 60 s; that node has the txID neither mined
    (`gettransactioninfobyid`, `gettransactionbyid`) nor pending (`gettransactionfrompending`);
    and that node's head is no more than 60 s behind this machine's clock. The output carries
    `asOf: {block, blockTime, host}` and says "this node returned no record of it as of block N on
    <host>; another node may have it. Sending this payment again is a second payment if this one
    was included";
  - when that node's head is more than 60 s behind this machine's clock: `status: "not found"`
    with `nodeBehindSecs` and "this node is <n> s behind; ask another node with `--rpc`", never
    `expired-unseen`;
  - a txID that is not found and not in the journal has no known expiration: `status: "not
    found"`.
  No message says or implies that sending again is safe. A repeat within 10 minutes still needs
  `--allow-duplicate`, and the duplicate refusal names the earlier txID and its `expiresAt` with
  "if it was included, this is a second payment; look it up first (`zwallet tx --chain tron
  <txid>`)". A wording test, as 04's round 8 test does for restore hints, refuses these phrases
  in `src/` and `README.md`, case-insensitive: "cannot double-pay", "can't double-pay", "no
  double payment", "cannot be paid twice", "safe to send again", "safe to resend", "safe to
  retry", "you can resend", "until then do not send", "after that, send", "can never be
  included", "was never included", "was not paid", "definitively". It does not scan this file,
  which quotes them.
- **Broadcast outcomes** (`Wallet.java:507-630`, E6), the request carrying the exact signed bytes:
  - `result: true` → sent. A `txid` in the reply other than the computed one →
    `broadcastWarning`, then the computed one is looked up.
  - `DUP_TRANSACTION_ERROR` → known: success with `broadcastWarning`.
  - `SIGERROR`, `CONTRACT_VALIDATE_ERROR`, `CONTRACT_EXE_ERROR`, `BANDWITH_ERROR`, `TAPOS_ERROR`,
    `TOO_BIG_TRANSACTION_ERROR`, `TRANSACTION_EXPIRATION_ERROR` → `RPC_ERROR` "the node rejected
    the transaction; nothing was broadcast". These are thrown by `pushTransaction` before the
    P2P broadcast (`Wallet.java:575-576`).
  - `NOT_ENOUGH_EFFECTIVE_CONNECTION`, `SERVER_BUSY`, `NO_CONNECTION`, `BLOCK_UNSOLIDIFIED`,
    `OTHER_ERROR`, a timeout or an HTTP error → the computed txID is looked up (mined, then
    pending); found = success with `broadcastWarning`; else `BROADCAST_UNCERTAIN` with
    `expiresAt`: "look it up with `zwallet tx --chain tron <txid>`; it can still be included
    until <expiresAt>. Sending this payment again is a second payment if this one was included".
    `NOT_ENOUGH_EFFECTIVE_CONNECTION` can come after the node already holds it (`Wallet.java:578-580`).
- **`--wait`** polls `gettransactioninfobyid` until `blockNumber` appears (`--wait-timeout`, 90 s
  default). Type 31: `success` = `receipt.result == "SUCCESS"`; any other value (`REVERT`,
  `OUT_OF_ENERGY`, …) is `success: false`. Type 1 has no `receipt.result` (M4): `success` =
  `gettransactionbyid` `ret[0].contractRet == "SUCCESS"`. Output adds `blockNumber`, `energyUsed`,
  `netUsage`, `feeSun`. A wait that ends unmined reports `pending`, `not found` or
  `expired-unseen` by the rules above.
- **Lookup answers about another transaction** → `RPC_ERROR`: zwallet hashes the returned
  `raw_data_hex` and compares it with the txID asked for.

### CLI surface

| Command | Tron behaviour |
|---|---|
| `chains` | a row `{key: "tron", name: "Tron", family: "tron", symbol: "TRX", chainId: 728126428, rpc}` |
| `address --chain tron [--verify]` | `{chain: "tron", key, address: "T…", verified, verifiedBy?, warning?}`; `--verify` adds `index` and `added?` |
| `addresses` | a `tron` row with `T…`; two QR codes when verified, the EVM one then the Tron one, each captioned |
| `balance --chain tron [--token usdt\|<T…>] [--rpc <url>]` | `{chain, key, address, symbol, decimals, balance, raw, activated, verified, verifiedBy?, warning?, token?}`; `--token usdt` takes the registry row (symbol and decimals from the registry); `--token <T…>` asks the contract, as EVM `--token` does |
| `balance` | a `tron` row among the mainnets: native TRX and the USDT token row, `address` on the row (it differs from the EVM address), the row's own `warning`; the QR after the networks stays the EVM one (`balance --chain tron` shows the Tron one) |
| `transfer --chain tron --to <@nick\|T…> --amount <dec> [--token usdt] [--dry-run] [--wait [--wait-timeout <dur>]] [--allow-duplicate] [--force] [--rpc <url>]` | output below. `--token` takes only `usdt` on Tron; any other value, a `T…` included, is `VALIDATION_ERROR` "only registry tokens are sent on Tron: --token usdt" before any RPC. A `T…` token is for `balance` only |
| `tx --chain tron <txid>` | `{chain, txHash, found, status: "mined"\|"pending"\|"not found"\|"expired-unseen", from, to, value, valueRaw, symbol, token?, explorerUrl, blockNumber?, success?, feeSun?, expiresAt?, asOf?, nodeBehindSecs?, hint?}` (§Send safety for `expired-unseen`); accepts the id with or without `0x` |
| `sign-message --chain tron`, `call`/`invoke --chain tron` | `CAPABILITY_UNSUPPORTED` |

`transfer --chain tron` output: dry-run `{dryRun: true, chain, key, from, to, toNickname?, amount,
amountRaw, symbol, decimals, token?, contractType: "TransferContract"|"TriggerSmartContract",
refBlock: {number, id}, timestamp, expiration, expiresAt, feeLimit?, fees, nativeBalance,
sufficientFunds, shortfall, duplicateSends?, whitelist, signerDecoded, txHash, recoveredFrom}`. No
`rawTx`: signed Tron bytes are broadcastable by anyone until they expire. Live: the same without
`dryRun: true` and `recoveredFrom`, plus `explorerUrl, journal, broadcastWarning?, rpcTxHash?`;
with `--wait` `{status, success, blockNumber, energyUsed?, netUsage?, feeSun, hint?}`.
`signerDecoded` = `{kind: "native-transfer", owner, to, amount}` or `{kind: "transfer-call",
token, owner, to, amount, feeLimit, assetType: "unverified"}`, plus `expiration` and `txHash`,
the EVM kind names reused.

- **Address cache.** `create` caches `addresses.tron` for new keys (`derive_addresses` derives once
  per family, `src/chains/mod.rs:210-229`). Keys made before this version have none.
  `address --verify` (password) then adds every chain this build derives that the cache lacks,
  under the writer lock, and reports it as `added: [{key, chain, now}]`, with no tamper warning
  (`cache_repairs` today looks only at entries that exist, `src/core/store.rs:705-735`). While
  locked and uncached, `address --chain tron` is `WALLET_LOCKED`: "unlock once, or run `zwallet
  address --chain tron --verify`".
- **Session verification after unlock.** `address --chain tron` while unlocked asks the daemon
  `address{chain: "tron"}`, checks the cache and the peer (`Daemon::address`,
  `src/commands.rs:561-565`): `verifiedBy: "session"` on macOS with no prompt, as for EVM. Linux
  stays unverified (02 §Address trust).
- **Address shape and comparison.** `address_shaped` (`src/commands.rs:255-267`) gets a Tron arm:
  34 characters, base58 alphabet, base58check valid, first byte `0x41`. Tron addresses compare
  exactly, never case-insensitively, in `cache_mismatch` (`src/commands.rs:366-376`),
  `cache_repairs` and the signer check of `sign_and_send` (`src/commands.rs:1707`).
- **Owner permission.** `balance --chain tron` reads `owner_permission` from `getaccount`. When it
  is not `{threshold: 1, keys: [this address, weight 1]}`, the output carries `warning`: "this Tron
  account's owner permission names other keys (…): funds here may not be controlled by this
  wallet's key; do not fund it". `transfer` refuses then (`VALIDATION_ERROR`), before signing.
- **QR** of a verified Tron address: the plain `T…`, captioned "scan to send to T…" and "(Tron
  only: TRX and TRC20 tokens; not an EVM address)". 34 bytes need QR version 3 (29 × 29 = 841
  modules), more than the 512-character budget any outside source has, so README's argument for
  the EVM code still holds.
- **Display integrity.** Base58 is printable ASCII, so `T…` passes the write point as is. A cached
  or relayed value that fails the Tron shape is `VALIDATION_ERROR`, shown escaped and cut at 64
  (`cached_address`, `reply_address`). Node strings (revert messages, `message` hex) count against
  the 512 "everything else from outside" budget, as RPC error text does now.
- **Explorer:** `https://tronscan.org/#/transaction/<txid>` and `…/#/address/<T…>` ([unverified],
  Q1).

### TUI

For the `03-tui.md` coverage matrix (owned by the parallel TUI task; this spec does not edit it):

| Capability | Inputs | Outputs | Confirmations | Errors to show |
|---|---|---|---|---|
| Tron address | key; chain `tron` in the chain picker | `T…`, trust level (session / locked / unverified) | none | `WALLET_LOCKED` for an old key with no cached Tron address (with the `--verify` command); `VALIDATION_ERROR` for a tampered cache |
| Balance | key | TRX; USDT; `not activated`; the owner-permission warning | none | `RPC_ERROR` (chain identity, 429 from a TronGrid `--rpc`); a USDT row error |
| Transfer TRX / USDT | payee from the checked list's address entries naming `tron` only (never `*`); asset TRX or USDT; amount (6 decimals) | review: from, `T…` and nickname, amount and symbol, bandwidth source and burn, activation fee, energy and burn, `feeLimit`, estimated and maximum cost in TRX (`estimatedCostSun`, `maxCostSun`), TRX left = balance − `maxCostSun` ("at least, at the prices of block N"), `expiresAt`; then the txID (64 hex) and status pending → mined / success / failed, or `expired-unseen` with its "this node returned no record of it as of block N on <host>" wording and no resend offer | the exact-intent confirm EVM uses | `WHITELIST_DENIED`, `WHITELIST_INVALID`, `FEE_CAP_EXCEEDED` (with the `fee-cap tron` command), `INSUFFICIENT_FUNDS` (TRX for fees, not activated), duplicate and send-lock `VALIDATION_ERROR`, owner-permission refusal, `RPC_ERROR` (simulation revert, node rejection), `BROADCAST_UNCERTAIN` (with `expiresAt`) |
| Tx detail | txID | status, from, to, amount, token, block, success, energy, bandwidth, fee burned, `expiresAt`; for `expired-unseen` its `asOf` (block, time, host) and the sentence "this node returned no record of it as of block N on <host>; another node may have it. Sending this payment again is a second payment if this one was included"; for a lagging node `nodeBehindSecs` and "ask another node" | none; never a resend offer | `RPC_ERROR` when the node answers about another transaction |
| History | key | the send journal's Tron rows (with `expiresAt`) | none | a note that Tron history is the journal only (no Tron explorer client is specified) |
| Whitelist | nickname, `T…` (chain fixed to `tron`); `fee-cap tron` amount in TRX | the summary with `T…` and its stored 0x body | the password, as for every edit | the parse-time refusals of §The whitelist |

## Open questions

| ID | Question | Why it is open | Closed by |
|---|---|---|---|
| Q1 | Tronscan's URL form `https://tronscan.org/#/transaction/<id>`, `/#/address/<T…>` | Tronscan is a script-rendered page; not fetched | TRON-7 opens one live URL and reads the page |
| Q2 | Which level other wallets (TronLink, Ledger Live) count for their second account | Not published in a source read here; the spec claims key 0 only (TronWeb's default path) | TR23: the owner imports the mnemonic in their Tron wallet and compares key 0 |
| Q3 | Does `POST /wallet/broadcasthex` work without a key on PublicNode and TronGrid? | Not called with a payload: this spec's brief allowed read-only calls only. GET → 405 shows the servlet is there (M9) | Before PR C starts, run by the dispatcher: one `POST /wallet/broadcasthex` per endpoint whose `transaction` is a serialized `Transaction {raw_data: 1}` with no `signature` field (not the bare `raw_data` hex, which would fail as a parse error and test nothing). No node can include it; java-tron refuses it, `TransactionCapsule.java:654-656` "miss sig or contract". A `SIGERROR` reply shows a keyless POST reaches validation; it is not evidence that a signed broadcast succeeds (that is TR23). A 401/403 means PR C needs another default |

Closed while writing: staked energy is spent before any TRX is burned
(`ReceiptCapsule.java:275-302`), and the energy a call may use is min(staked + balance ÷ price,
`fee_limit` ÷ price) (`VMActuator.java:583-621`). So `fee_limit` bounds the call's whole energy,
staked included, and the TRX it burns for energy is at most `fee_limit`. The CLI's `fee_limit`
therefore covers `energy_used` × price even when staked energy will pay.

## Acceptance

| ID | Run | Observe |
|---|---|---|
| TR1 | derive from the A1 mnemonic | i=0 `TWer2Ygk5TEheHp3TPuYeqxmB6SsGZmaL6`, i=1 `TPjjvMwjPoDC32V2dGDYTkLH4E5LAtBZ6C` (`m/44'/195'/0'/0/i`) |
| TR2 | `create a`, then `address --chain tron` locked and unlocked | `addresses.tron` cached; locked `verified: false`; unlocked (signed macOS build) `verifiedBy: "session"` with no prompt; a QR only when verified |
| TR3 | a wallet whose key has no `addresses.tron`: `address --chain tron`, then `address --chain tron --verify` | locked: `WALLET_LOCKED` naming unlock and `--verify`; `--verify`: `added: [{key, chain: "tron", now}]`, no tamper warning; then cached |
| TR4 | cache `addresses.tron` with one letter's case flipped; with a bad checksum; with a `0x` value; and compare two addresses that differ only in case | each cache `VALIDATION_ERROR`, the value escaped, never shown as an address; all 28 single-letter case flips of `TWer2…` fail base58check (checked 2026-10-28, M11); the comparison is byte equality, false for the case variant without parsing |
| TR5 | `balance --chain tron --rpc <mock whose block 0 is Nile's>` | `RPC_ERROR` naming the genesis mismatch; no balance read |
| TR6 | mock: the M7 `getaccount` answer recorded as a fixture; live (`--ignored`): cache `addresses.tron` = `TWer2…`, `balance --chain tron --json` | mock: `warning` names `TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk`. Live: `raw` equals the same node's `getaccount.balance` read in the test; `warning` is present exactly when that live `owner_permission` is not this address alone with threshold 1. Neither the balance nor the key is pinned |
| TR7 | live and mock: `balance --chain tron --token usdt`; a mock `balanceOf` error | USDT, decimals 6 from the registry, `raw` = `balanceOf`; the mock error is the token row's error, TRX still shown |
| TR8 | `balance` (every network) | a `tron` row among the mainnets with TRX and USDT; Sepolia still last; the row has its own `address` |
| TR9 | `whitelist add bob <T…>` (password) | summary shows `T…` and its stored `0x` body, chains `tron`; `whitelist.json` stores `0x…` with `["tron"]`; `list --json` shows `address: "T…", family: "tron"`; `--verify` gives `verified: true` |
| TR10 | `add` with `T…` and `--chains base`; `T…` and `--chains '*'`; `0x…` and `--chains tron`; `--chains tron,base`; a bad checksum; `--contract` on `tron`; and `validate` on a hand-built list with `chains: ["tron", "base"]` or a contract entry on `tron` | each `VALIDATION_ERROR` before the password, nothing written; `validate` refuses both lists |
| TR11 | a list signed before this version (gen ≥ 1, EVM entries, one `*` entry) | verifies, unchanged bytes; the golden-bytes test passes untouched; the `*` entry is no payee on `tron` (`payee` with `wildcard: false`) and still one on every EVM chain; an older build's `check_shapes` accepts a list with a `tron` entry; a new-build regression of the old meaning (not a run of an old binary): the EVM `check_tx`, whose `payee` path is `covers_chain` as on `main`, refuses a native transfer and a registry `transfer` on `base` to the EVM twin `0x<body>` of a `chains: ["tron"]` entry with `WHITELIST_DENIED` and no `command`, and so does the daemon's `sign_tx` |
| TR12 | decode the golden vectors; sign one with the test key | fields as §The transaction; re-encode is byte-equal; txID = sha256; the signature recovers to the key; `v` ∈ {0, 1}; low `s`; both mainnet signatures recover to their owners |
| TR13 | `tron::tx::decode_raw` on bytes with `raw_data` field 10 (memo), 9, 12 or 3 (`ref_block_num`); a field twice or out of order; trailing bytes; two contracts; type 46, 2, 30, 54; `Contract` field 3, 4 or 5 (`Permission_id` 2); an `Any` field 3; a wrong `type_url`; `TransferContract` field 4; `TriggerSmartContract` `call_value` (its field 3), `call_token_value` (5), `token_id` (6) or field 7; a wrong wire type; a non-minimal varint | each `VALIDATION_ERROR` "refusing to sign"; type 46's message says it hands the account to other keys |
| TR14 | daemon `sign_tx` with a verified list: TRX and USDT to `bob` (tron); TRX to an unlisted `T…`; to an entry for `base` only; to a `*` entry's body; to self; USDT to the USDT contract; a non-registry TRC20 `transfer`; `approve`; owner = another key; `NoList`; `Unusable`; and every TR13 input through `sign_tx` | the first two signed; then `WHITELIST_DENIED` with `whitelist add <nick> T… --chains tron`; `WHITELIST_DENIED` without `command` ×6; `VALIDATION_ERROR`; `WHITELIST_INVALID` (with the add command); `WHITELIST_INVALID` (no command); `VALIDATION_ERROR`; zero signatures for every refusal (daemon-side count) |
| TR15 | `fee_limit` above the cap; a list with no `tron` cap (a TRX send, then a USDT send); `fee_limit` on type 1; expiration > now + 5 min; expiration ≤ timestamp | `FEE_CAP_EXCEEDED` with `fee-cap tron <fee_limit in TRX>`, `feeWei`, `capWei`; `FEE_CAP_EXCEEDED` with `fee-cap tron 30` (TRX), then `fee-cap tron <max(fee_limit, 30)>`; `VALIDATION_ERROR` ×3 |
| TR16 | a fake daemon replies with a `txHash` ≠ sha256(`rawData`), with `rawData` ≠ the bytes sent, or with a signature from another key | `VALIDATION_ERROR`, nothing broadcast, no journal line |
| TR17 | mock: `transfer --chain tron --to @bob --amount 1 --dry-run`; and to an unlisted `T…`; TRX to a payee that does not exist with prices unlike M5 (P_sys 2 TRX, P_ca 0.3 TRX, P_tx 2,000 sun); the same with staked bandwidth that covers `bytes` × R; TRX to a payee whose `getaccount` says `type: "Contract"` | `fees`, `feeLimit`, `whitelist: {allowed: true}`, `txHash`, `recoveredFrom`, no raw bytes, no broadcast; the unlisted one: `whitelist: {allowed: false, …}` and no signature request; the new payee: `maxCostSun` = amount + max(2,300,000, `bytes` × 2,000) and `estimatedCostSun` = amount + 2,300,000, from the prices read; a balance between amount + 1,100,000 (M5's figure) and that maximum is `INSUFFICIENT_FUNDS`; with staked bandwidth, `estimatedCostSun` = amount + 2,000,000 while `maxCostSun` is unchanged; the contract payee: `maxCostSun` = amount + max(P_sys + P_ca, `bytes` × P_tx) |
| TR18 | mock: TRX balance below amount + bandwidth + activation; USDT with too little TRX for energy; a TRX balance above `estimatedCostSun` but below `maxCostSun` (no staked energy, `fee_limit` = 1.2 × the estimate); an unactivated sender; a simulation that reverts | `INSUFFICIENT_FUNDS` ×4 (the third names `maxCostSun` and `shortfall` against it) and `RPC_ERROR`, each before signing (zero `SignTx`) |
| TR19 | `transfer --chain tron` with `--nonce`, `--replace`, `--bump`, `--allow-pending`, `--max-fee-gwei` | `VALIDATION_ERROR` before any RPC |
| TR20 | mock: TRX and USDT to `@bob` with `--wait`; a USDT receipt whose energy is above the simulation but within `fee_limit`; one with `OUT_OF_ENERGY` | `broadcasthex` bytes decode to the signed transaction; txHash = sha256(raw_data); one journal line with `expiresAt`; USDT receipt `SUCCESS` with all-zero `contractResult` → `success: true`; a `REVERT` receipt → `success: false`; the higher-energy receipt → `success: true`, `feeSun` above `estimatedCostSun` and within `maxCostSun`; a TRX send to a new payee whose receipt burns P_ca though staked bandwidth covered it at the preflight → within `maxCostSun`; `OUT_OF_ENERGY` → `success: false` with `feeSun` and a message that the fee was burned |
| TR21 | mock: `DUP_TRANSACTION_ERROR`; `CONTRACT_VALIDATE_ERROR`; a reply with another `txid`; a timeout; a lookup answer whose `raw_data` hashes to another id; a repeat within 10 min | success with `broadcastWarning`; `RPC_ERROR` nothing broadcast; `broadcastWarning` and the computed txID looked up; `BROADCAST_UNCERTAIN` with `expiresAt` and the second-payment sentence; `RPC_ERROR`; the duplicate refusal naming the earlier txID, its `expiresAt` and the second-payment sentence |
| TR25 | mock, `tx --chain tron <id>` and `--wait`: (a) the tx was included before its expiration but the node answers `{}` while its head is before `expiresAt` + 60 s; (b) the node's head is 90 s behind this machine's clock and past `expiresAt` + 60 s per the local clock only; (c) this machine's clock is past `expiresAt` + 60 s, the node's head is not; (d) a journal txID, the node's head 2 min past `expiresAt` and within 60 s of the local clock, nothing mined or pending; (e) an id not in the journal, nothing found | (a) `not found`; (b) `not found` with `nodeBehindSecs`; (c) `not found`; (d) `expired-unseen` with `asOf: {block, blockTime, host}`, "this node returned no record of it as of block N" and "a second payment if this one was included"; (e) `not found`. Never `expired-unseen` in (a)–(c) or (e). A wording test over the code and the docs finds no text that calls a resend safe |
| TR22 | the TUI coverage matrix | every row of §TUI is a row there (closed by the TUI task) |
| TR23 | **owner-gated.** The owner funds the key's Tron address with a little TRX and USDT, adds a payee, and decides to send. Then `transfer --chain tron --to @<nick> --amount <small> --wait`, TRX then USDT | both `success: true`; Tronscan shows both; the owner's own Tron wallet shows key 0's address from the mnemonic (Q2); `broadcasthex` carried both sends |
| TR24 | `sign-message --chain tron`; `call --chain tron …`; `invoke --chain tron …` | `CAPABILITY_UNSUPPORTED` |

## Review round 1 (2026-10-28): what changed

Three-engine review: grok-4.7 APPROVE, gpt-6-astra REJECT, fable REJECT.
1. `expired` was a proof of non-payment taken from the local clock. Now `expired-unseen`, decided
   by the queried node's head block, worded as an observation, never offered as a reason to send
   again (§Send safety, TR25).
2. `maxCostSun` showed the simulated cost. Now `estimatedCostSun` and `maxCostSun` (the signed
   `fee_limit` plus worst-case bandwidth and activation) are separate, and the balance check uses
   the maximum (§Fees, TR18, TR20).
3. The case-variant brute force is gone: the checksum and the exact comparison are tested apart
   on fixed inputs (TR4, M11).
4. Also: TR10 adds `T…` with `*`; `validate` refuses mixed-family and Tron contract entries
   (§The whitelist); EVM twin regression (TR11); nested strict decode and inner `call_value`
   (§The daemon's judgement, TR13); golden vectors pin the codec only; `transfer --token` is
   `usdt` only on Tron; TR6 pins nothing live; README and release note say `*` no longer covers
   `tron`; Q2 claims key 0 only; Q3 closes before PR C; Q4 closed from java-tron's energy code.
5. PRs: A (see it), B1 (codec), B2 (guard), C (send it), no line cap
   ([`v0-3-tron/units.md`](v0-3-tron/units.md)).

## Review round 2 (2026-10-28): what changed

grok-4.7 APPROVE, fable APPROVE, gpt-6-astra REJECT on one residual of round 1's F1.
1. Activation in `estimatedCostSun` and `maxCostSun` uses the prices this preflight reads, never
   M5's constants. `maxCostSun` assumes staked bandwidth is gone, and treats a payee that is
   missing or a contract (which can self-destruct before the block) as able to take either
   bandwidth branch. The new-account bandwidth bill replaces the normal one (java-tron cited), so
   the maximum takes the larger branch, not a sum (§Fees, TR17, TR20).
2. Non-blocking: the wording test's banned phrases are listed (§Send safety); the `maxCostSun`
   balance check is stricter than the node, on purpose (§Fees); TR11 is a new-build regression of
   the old meaning; Q3's POST carries a `Transaction` envelope without a signature, and its reply
   is not evidence of a signed broadcast; "this node returned no record of it as of block N"; the
   TUI's "TRX left" names its prices' block; the TUI's tx detail shows `asOf`.

## Evidence

| ID | Source |
|---|---|
| E1 | SLIP-44, `slip-0044.md:228` `\| 195 \| TRX \| Tron \|`, satoshilabs/slips@570ed55b7fde158f1116be34fc2faa35dada5912 |
| E2 | TronWeb, tronprotocol/tronweb@974ea0bcd98c2a10f2e94addd55d4fa3b199f98a: `src/utils/constants.ts:6-7`, `src/utils/accounts.ts:40-50` |
| E3 | Ledger Tron app, LedgerHQ/app-tron@a0f3aebf51202e8587668d317c887b6f26838d1c, `Makefile:43` `PATH_APP_LOAD_PARAMS = "44'/195'"` |
| E4 | `node vec-tronweb.cjs` and `node vec-scure.cjs` (scratch), 2026-10-28: both print the TR1 table |
| E5 | tronprotocol/protocol@8432beca98c52ba04c82aea0b48ae9f9f882c82a: `core/Tron.proto:336-384` (Contract), `:431-445` (raw); `core/contract/balance_contract.proto:32-36`; `core/contract/smart_contract.proto:74-81`; `api/api.proto:653-675` (Return codes) |
| E6 | java-tron, tronprotocol/java-tron@b33eed89a6a424c498d4fb1b03ca2c86eddf4840: `chainbase/.../TransactionCapsule.java:468-492, 519-526, 584-587, 654-656, 721-725`; `chainbase/.../ReceiptCapsule.java:275-302`; `actuator/.../VMActuator.java:583-621`; `crypto/.../Rsv.java:17-24`; `crypto/.../ECKey.java:923-941, 978-986`; `framework/.../db/Manager.java:798-822, 853`; `common/.../Constant.java:19, 29`; `actuator/.../TransferActuator.java:45-57, 111-131`; `actuator/.../vm/repository/RepositoryImpl.java:488-491`; `chainbase/.../db/TransactionTrace.java:373-378`; `chainbase/.../db/BandwidthProcessor.java:116-128, 139-152, 160-170, 192-262, 259-287`; `framework/.../Wallet.java:507-630, 1855-1870`; `framework/.../http/BroadcastHexServlet.java:23-43`; `framework/.../http/FullNodeHttpApiService.java:464, 495`; `framework/.../jsonrpc/TronJsonRpcImpl.java:426-427, 1369-1371` |
| E7 | tokio-rs/prost@79ed991edf4f8451c57b89c68b5bce8e53cde367 `prost-derive/src/lib.rs:191`; crates.io: prost 0.14.4 (2026-06-07) |
| E8 | https://tether.to/en/supported-protocols, fetched 2026-10-28 |
| E9 | https://www.circle.com/blog/circle-is-discontinuing-support-for-usdc-on-the-tron-blockchain (2024-02-21) and https://developers.circle.com/stablecoins/usdc-contract-addresses, fetched 2026-10-28 |
| E10 | https://developers.tron.network/reference/rate-limits, fetched 2026-10-28 |

Measurements (2026-10-28, `curl`, both endpoints unless noted):
- **M1** 03:13:58Z–03:16:51Z: the endpoint table and the burst above.
- **M2** `balanceOf`, `decimals()` → `…06`, `symbol()` → `USDT` on `TR7NHq…`.
- **M3** 03:15:22Z: `transfer(TAAimb…, 1 USDT)` from `TPdMMF…` → `energy_used` 64,285
  (`energy_penalty` 49,635); to `TPjjvM…` → 130,285 (100,635). `estimateenergy` → "this node does
  not support estimate energy".
- **M4** block 87494658: `21b776c7…a690` (USDT; receipt `SUCCESS`, `energy_usage_total` 130,285,
  `energy_penalty_total` 100,635, `net_usage` 345, `contractResult` 32 zero bytes, `v = 0x1b`) and
  `e4e8962a…f801` (TRX 1.5; `net_usage` 267, no `receipt.result`, `ret[0].contractRet`
  `SUCCESS`, `v = 0x00`, high `s`); `sha256(raw_data)` = txID for both; both signatures recover to
  their owner (`@noble/curves`).
- **M5** 03:15:24Z `getchainparameters`: the §Fees table; `getMemoFee` 1,000,000.
- **M6** 03:20:25Z `getblockbynum 0` (mainnet and Nile ids above); `/jsonrpc eth_chainId` →
  `0x2b6653dc`.
- **M7** `getaccount TWer2…`: balance 8,000,010; `owner_permission.keys` =
  `[TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk]`; 7 `assetV2` entries.
- **M8** `getaccount` of a random never-used address → `{}`; `getaccountresource` → `{}`;
  `balanceOf` with it as `owner_address` → 0.
- **M9** `GET /wallet/broadcasthex` and `/wallet/broadcasttransaction` → 405; `GET
  /wallet/nonexistentpath` → 404.
- **M10** 03:15:25Z `createtransaction` and `triggersmartcontract` (unsigned, not broadcast): the
  golden vectors; `encode_raw.py` prints `mine == node raw_data_hex: True | sha256 == txID: True`
  for both.
- **M11** 2026-10-28, `caseflip.cjs` (scratch, `@scure/base` 1.2.6): every single-letter case
  flip inside the base58 alphabet, 28 for `TWer2…` and 25 for `TPjjv…`, decodes as base58 and
  fails the checksum.
