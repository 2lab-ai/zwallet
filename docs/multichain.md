# One seed, four chain families

[README](../README.md) · [Commands](commands.md) · [Security threat model](security-threat-model.md)

The same encrypted BIP39 recovery phrase derives every named account on EVM, Tron,
Solana and TON. Adding the two Ed25519 families does not replace the seed or change
existing EVM/Tron addresses. Each chain still has its own address; sending to an
address from another family is not a cross-chain transfer.

| Family | Path for key index `i` | Address / supported value transfer |
|---|---|---|
| EVM | `m/44'/60'/0'/0/i` | EIP-55; native and registry ERC-20, existing contract policy |
| Tron | `m/44'/195'/0'/0/i` | base58check `T…`; TRX and registry TRC20 USDT |
| Solana | `m/44'/501'/i'/0'` | case-sensitive base58 Ed25519 public key; native SOL only |
| TON | `m/44'/607'/i'` | V4R2 contract, workchain 0, subwallet ID 698983191; native TON only |

Solana and TON use BIP39 with the empty passphrase followed by fully hardened
SLIP-0010 Ed25519 derivation. TON addresses are derived from the complete wallet
contract StateInit, not directly from the public key. zwallet stores/displays the
canonical raw `0:<64 hex digits>` form and accepts mainnet friendly bounceable and
non-bounceable addresses with valid checksums. A friendly address's bounce flag is
not a different account identity; zwallet's outgoing transfer shape fixes its own
bounce policy.

This TON scheme is the **multichain BIP39 scheme**. A TON-native 24-word mnemonic
uses a different derivation, even when it uses the same word list. Do not assume
pasting the same phrase into another wallet yields the same address. Recovery needs
the scheme, path/index, contract version, workchain and subwallet ID above. The
existing default word count and accepted BIP39 word counts are preserved; no new
mnemonic-import command is introduced. Backup remains an interactive CLI operation.
See the [TON mnemonic specification](https://docs.ton.org/contracts/standard/wallets/mnemonics)
and [SLIP-0010](https://github.com/satoshilabs/slips/blob/master/slip-0010.md).

## Existing wallets

Old wallet files can lack Solana/TON address caches. While locked, a missing address
is an explicit error, never an EVM address reused under a new chain name. A
password-verified operation derives the address from the existing seed and repairs
the cache. Locked cached addresses remain unverified. The live daemon derives its
own address and a conflicting cache is refused.

```sh
zwallet address --chain solana --verify
zwallet address --chain ton --verify
zwallet balance --chain solana
zwallet balance --chain ton
zwallet tui --chain tron
```

On the TUI Receive tab, `Q` opens the locally verified QR and full address together.
The Tron address fits beside the QR at both 80×24 and 120×40. Longer addresses may
wrap in the separate text column. The rendered QR keeps its quiet zone. A cached,
unverified or impostor-supplied address cannot produce a QR.

## Native sends

Add recipients with an explicit chain. Native amounts have 9 decimal places on
both new families. Named keys keep the same creation index across families.

```sh
zwallet whitelist add recipient <solana-address> --chains solana
zwallet whitelist native-cap solana 0.1
zwallet transfer --chain solana --to @recipient --amount 0.001 --dry-run

zwallet whitelist add recipient-ton <ton-address> --chains ton
zwallet whitelist native-cap ton 1
zwallet transfer --chain ton --to @recipient-ton --amount 0.01 --dry-run
```

Whitelist changes require the password. A dry-run can obtain a signature for
verification but prints no broadcastable signed transaction. The TUI selects a
password-checked payee, reviews exact intent and fee information, then requires
the nickname separately. A changed review must be confirmed again.

The daemon decodes only the supported canonical transfer bytes before checking the
actual recipient, amount and exact-chain whitelist entry. Solana accepts one legacy
System Program transfer, one signer, no extra program/priority-fee instruction or
lookup tables. TON accepts a bounded V4R2 transfer shape, not plugin management,
carry-all/destroy modes or arbitrary contract calls. Tagged signed identities keep
case-sensitive addresses distinct; EVM `*` entries grant no Solana/TON permission.
Old whitelist bytes remain valid unchanged. Older binaries reject the newly tagged
address format, so downgrading after adding these payees is not supported.

SPL tokens, Jettons, arbitrary program/contract invocation, message signing, staking,
NFTs and EVM nonce/replacement flags are unsupported for these families. Task
reports/`--task` accounting currently remain EVM/Tron capabilities. These omissions
are explicit capability errors, not silent fallbacks.

## Fees, network evidence and retries

Solana's supported one-signature transaction has the current 5,000-lamport base fee.
The CLI checks the RPC estimate agrees and checks blockhash validity before
broadcast. A legacy message encodes neither a maximum fee nor a chain ID: the
5,000-lamport policy check is a protocol assumption, not a signature committing to
a fee ceiling. Genesis checks are evidence from the selected RPC, not a proof against
a compromised node. Priority fees are not supported.

TON V4R2 encodes no maximum fee or global network ID. The daemon enforces the
supported operation, recipient, amount, subwallet and bounded expiry; a reserve and
RPC fee estimate cannot enforce an on-chain maximum charge. Its network check uses
the mainnet zero-state identity reported by the RPC. A signature may be replayable
on another network with the same key/contract/seqno: this implementation does not
claim cryptographic cross-network separation. Node/indexer observations may be
wrong or delayed. A changed fee estimate causes a review change where applicable,
but no estimate is a guarantee.

TON's `txHash` submission identifier is a signed-message-body hash, marked by
`hashKind`, not a blockchain transaction hash. Lookup resolves it using matching
message evidence. A seqno increase alone does not prove a payment: the wallet may
consume a seqno while an outgoing action fails. Only the matching outgoing transfer
supports a successful payment observation; incomplete evidence remains unknown.
The full chain transaction hash is distinct when available.

The per-account send lock and last-1-MiB journal duplicate warning are local aids.
A transport failure after broadcast may mean the payment happened. Read the
returned identifier with `zwallet tx --chain <chain> <id>` before trying again.
Neither family provides the EVM fixed-nonce replacement workflow here. Rebuilding a
transfer with a new blockhash/seqno may pay twice. Existing cumulative-budget and
policy-rollback limits remain unchanged.

## Verification and delivery

The [tracked contract](../.prd/11-multichain.md) and
[completion checklist](../.prd/multichain/completion-checklist.md) map source and
regression evidence to the receive layout, derivation, signing and native workflows.
Fixtures are generated from public test inputs with independent Solana and TON
SDKs; live funded acceptance is not claimed. No owner's wallet or real funds are
used by the test suite.
