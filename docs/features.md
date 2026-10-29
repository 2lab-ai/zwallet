# Feature status

[Back to README](../README.md) · [Command reference](commands.md) · [Development](development.md)

This page is for people deciding whether zwallet fits their workflow and contributors
checking what still needs to be built. Reviewed against the handoff implementation on
2026-10-29. “Implemented” means a reachable code path exists in this tree; it
does not mean every published binary includes it or every public RPC is reachable.

![WIP](https://img.shields.io/badge/status-WIP-yellow) marks planned or incomplete behavior that you cannot use yet. A CLI
implementation does not make the corresponding TUI workflow implemented.

## Implemented features

| Capability | Current behavior and limits | Implementation and checks |
|---|---|---|
| Local key custody | Encrypted BIP39 master seed, named EVM keys, password-based verification, terminal-only phrase backup. No import command. | [commands](../src/commands.rs), [store](../src/core/store.rs), [CLI tests](../tests/cli.rs) |
| Timed signing | Human unlock, expiry and lock-all. Wallet/socket inspection uses bounded workers and deadlines; expiry and lock remain independent. Daemon makes no RPC calls. | [daemon](../src/daemon/mod.rs), [protocol](../src/core/protocol.rs), [CLI tests](../tests/cli.rs) |
| Eight EVM networks | Seven mainnets plus Sepolia. `Family` has only an EVM variant. | [chain registry](../src/chains/mod.rs), [offline EVM tests](../tests/evm_offline.rs) |
| Balances and read calls | Native/ERC-20 balances, all-network registry balances and ABI-decoded `eth_call`. Arbitrary token reads do not grant transfer permission. | [RPC](../src/chains/evm/rpc.rs), [ABI](../src/chains/evm/abi.rs), [live read/dry-run tests](../tests/live.rs) |
| Transfers and contract writes | Native/registry-token transfers; `invoke` by signature or raw calldata; policy-controlled signing; dry-run, broadcast and receipt waiting. Non-zero approvals and `transferFrom` are refused. | [commands](../src/commands.rs), [transactions](../src/chains/evm/tx.rs), [policy](../src/chains/evm/policy.rs), [CLI tests](../tests/cli.rs) |
| Signed whitelist | Password-authorized payees, contracts/selectors, execution-fee caps and message-signing toggle. | [whitelist commands](../src/commands/whitelist.rs), [daemon policy tests](../tests/whitelist_daemon.rs), [seed tests](../tests/whitelist_seed.rs) |
| Per-transaction limits | Optional signed native-value, direct registry-token amount and calldata byte caps. Enforced by the daemon on decoded bytes; not a cumulative budget or a constraint on indirect contract effects. | [commands](commands.md#per-transaction-limits), [policy](../src/chains/evm/policy.rs), [integration tests](../tests/policy_limits.rs) |
| Receipt deadline | `--wait-timeout` bounds in-flight receipt RPC calls and polling sleeps after broadcast. Expiry returns pending, and late receipts are not accepted. | [RPC](../src/chains/evm/rpc.rs), [deadline tests](../tests/review_receipt_deadline.rs) |
| Token registry | 19 deployments across seven networks: USDC, USDT, DAI and WETH where listed. BSC has none. Registry additions require a code change. | [exact deployments and tests](../src/chains/evm/registry.rs) |
| Message signing | EIP-191 `sign-message`, only when explicitly allowed by the whitelist. | [CLI](../src/cli.rs), [daemon](../src/daemon/mod.rs), [policy tests](../tests/whitelist_daemon.rs) |
| Send recovery | Journal, one-send lock, pending/recent-duplicate guards, fixed-nonce replacement and hash lookup. Duplicate matching is not calldata-aware. | [send guards and journal](../src/chains/mod.rs), [CLI tests](../tests/cli.rs) |
| CLI integration | JSON results/errors, terminal address QR codes only for verified addresses. TUI rejects `--json`; backup remains human-only. | [CLI](../src/cli.rs), [QR](../src/qr.rs), [CLI tests](../tests/cli.rs) |
| Terminal navigation and reads | Seven tabs, a 55-action menu, one/all-network balances, session RPC selection, full receive addresses, password verification and cache repair, timed unlock and session details. Future actions stay explicitly disabled. Selecting/opening a key does not change the globally active key. | [state and tests](../src/tui/app.rs), [event loop](../src/tui/mod.rs), [terminal tests](../tests/tui.rs) |
| EVM history and transaction detail | Up to ten journal/explorer rows, pending first, explicit source coverage, hash lookup, fee/L1 fee, confirmations and registry-token receipt movements. Explorer coverage is limited to Ethereum, Optimism and Sepolia. Node/explorer content is a claim; nicknames require the session's checked list. | [history model](../src/tui/app/history.rs), [renderer](../src/tui/ui/history.rs), [UI tests](../tests/tui_history.rs), [backend tests](../tests/history.rs) |

| Terminal transfer flow | Send tab: native or non-zero registry token, checked payee, exact amount/fee review, nickname confirmation, pending/repeat overrides, journal hash and receipt deadline with typed extension. Requires a password-checked key/list and warm session. Replacement controls are still disabled in this unit. Real PTY acceptance and the whole local gate pass. | [transfer state](../src/tui/app/transfer.rs), [display](../src/tui/ui/transfer.rs), [21 mock tests](../tests/transfer_review.rs), [handoff](../.prd/research/handoff-tui-send-2026-10-29.md) |

## Planned features

| Feature | Status | Evidence and current boundary |
|---|---|---|
| Full TUI workflows | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [TUI design](../.prd/03-tui.md), [mockups](../.prd/03-tui-screens.md), [work units](../.prd/v0-2-tui/units.md). Navigation, balances, receive, verification, repair and session controls have reachable handlers. Password-verified family addresses stay pinned for the TUI lifetime. Whitelist forms, key creation, contract calls and message signing remain WIP. EVM history/detail and fixed-nonce replacement controls are implemented; replacements require a password-pinned key and authenticated original intent. The TUI-3+4 receipt records the still-pending process acceptance gates. |
| Tron (TRX / TRC20 USDT) | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [v0.3 design](../.prd/05-tron.md). No Tron family, registry entry, address derivation, RPC path or signer exists in `src/chains`. `--chain tron` is unsupported. |
| Solana and TON | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Architecture extension plans](../.prd/02-architecture.md). No family implementation or chain entry. |
| Cumulative spending budgets (rolling 24-hour native and per-token) | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Per-transaction limits do not track total spending; repeated transfers can still spend the full balance ([#26](https://github.com/2lab-ai/zwallet/issues/26)). |
| Calldata-aware duplicate detection for `invoke`; configurable duplicate window | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Current matching uses chain, kind, recipient and amount (plus token for token transfers), over a fixed ten-minute window. |
| User-added transfer tokens | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Reading a token by contract address is already available. |
| Secure Enclave / hardware whitelist key | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Today's whitelist authority derives from the wallet password; it is not a second factor. |

## Unsupported is not always planned

There is no zwallet import/restore or password-change command, EIP-712 typed-data
signing, Windows build, swap/bridge workflow, WalletConnect or ENS resolution in the
reviewed CLI. Do not infer a delivery commitment from an absent feature. The WIP table
above lists work supported by the repository's plans.

## Keeping the status accurate

When adding a feature, check its public command or UI path, its policy restrictions
and tests before moving it out of WIP. Update the README and this page together.
Keep design mockups labeled as plans until the corresponding screens can be reached.
Original requests and historical ledgers under `.prd/` remain historical evidence.


TUI fixed-nonce recovery is implemented in
[the replacement boundary](../src/wallet/replacement.rs) and
[the replacement state machine](../src/tui/app/replacement.rs). Speed-up and
cancel require a password-pinned owner and an authenticated original; uncertain
retry additionally requires this session's retained review. Native, registry
ERC-20 and allowed contract-call intents retain their original nonce and kind.
[The focused signing and UI regressions](../tests/replacement_review.rs) count
signature requests; [the PTY acceptance cases](../tests/tui_replacement.rs) count
requests at a proxy of an isolated test daemon.
