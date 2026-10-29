# Feature status

[Back to README](../README.md) · [Command reference](commands.md) · [Development](development.md)

This page is for people deciding whether zwallet fits their workflow and contributors
checking what still needs to be built. Reviewed on 2026-10-30 against the integration
of the TUI v2 and Tron handoffs (#35, #37–#49) with main `2440ef2`.
“Implemented” means a reachable code path exists in this source revision; it
does not mean every published binary includes it or every public RPC is reachable.

![WIP](https://img.shields.io/badge/status-WIP-yellow) marks planned or incomplete behavior that you cannot use yet. A CLI
implementation does not make the corresponding TUI workflow implemented.

## Implemented features

| Capability | Current behavior and limits | Implementation and checks |
|---|---|---|
| Local key custody | Encrypted BIP39 master seed, named keys with EVM and Tron addresses, password-based verification, terminal-only phrase backup. No import command. | [commands](../src/commands.rs), [store](../src/core/store.rs), [CLI tests](../tests/cli.rs) |
| Timed signing | Human unlock, expiry and lock-all. Wallet/socket inspection uses bounded workers and deadlines; expiry and lock remain independent. Daemon makes no RPC calls. | [daemon](../src/daemon/mod.rs), [protocol](../src/core/protocol.rs), [CLI tests](../tests/cli.rs) |
| Eight EVM networks | Seven mainnets plus Sepolia; EVM signing and contract calls. | [chain registry](../src/chains/mod.rs), [offline EVM tests](../tests/evm_offline.rs) |
| Balances and read calls | Native/ERC-20 balances, all-network registry balances and ABI-decoded `eth_call`. Arbitrary token reads do not grant transfer permission. | [RPC](../src/chains/evm/rpc.rs), [ABI](../src/chains/evm/abi.rs), [live read/dry-run tests](../tests/live.rs) |
| Tron addresses and balances | Derives `m/44'/195'/0'/0/i`; exact-case base58check addresses, genesis-checked TRX and TRC20 balances, USDT registry, owner-permission warnings. Locked caches remain unverified. | [Tron family](../src/chains/tron), [mock node checks](../tests/tron_offline.rs) |
| Tron transfers and lookup | TRX and registry USDT, explicit Tron payees, strict independently decoded signatures, dynamic fee maximum, journal expiration and fresh-node `expired-unseen` observations. No nonce, arbitrary calls, memos, permission changes or message signing. Funded live acceptance remains owner-run. | [preflight](../src/chains/tron/send.rs), [broadcast and lookup](../src/chains/tron/lookup.rs), [offline tests](../tests/tron_send.rs), [evidence](../.prd/v0-3-tron/c-evidence.md) |
| Transfers and contract writes | Native/registry-token transfers; `invoke` by signature or raw calldata; policy-controlled signing; dry-run, broadcast and receipt waiting. Non-zero approvals and `transferFrom` are refused. | [commands](../src/commands.rs), [transactions](../src/chains/evm/tx.rs), [policy](../src/chains/evm/policy.rs), [CLI tests](../tests/cli.rs) |
| Signed whitelist | Password-authorized payees, contracts/selectors, execution-fee caps and message-signing toggle. | [whitelist commands](../src/commands/whitelist.rs), [shared edit boundary](../src/wallet/whitelist.rs), [daemon policy tests](../tests/whitelist_daemon.rs), [seed tests](../tests/whitelist_seed.rs) |
| Per-transaction limits | Optional signed native-value, direct registry-token amount and calldata byte caps. Enforced by the daemon on decoded bytes; not a cumulative budget or a constraint on indirect contract effects. | [commands](commands.md#per-transaction-limits), [policy](../src/chains/evm/policy.rs), [integration tests](../tests/policy_limits.rs) |
| Receipt deadline | `--wait-timeout` bounds in-flight receipt RPC calls and polling sleeps after broadcast. Expiry returns pending, and late receipts are not accepted. | [RPC](../src/chains/evm/rpc.rs), [deadline tests](../tests/review_receipt_deadline.rs) |
| Token registry | 19 EVM deployments across seven networks, plus USDT on Tron. BSC has none. Registry additions require a code change. | [exact deployments and tests](../src/chains/evm/registry.rs) |
| Message signing | EIP-191 `sign-message`, only when explicitly allowed by the whitelist. | [CLI](../src/cli.rs), [daemon](../src/daemon/mod.rs), [policy tests](../tests/whitelist_daemon.rs) |
| Send recovery | Journal, one-send lock, pending/recent-duplicate guards, EVM fixed-nonce replacement and hash lookup; Tron uses expiration observations. Duplicate matching is not calldata-aware. | [send guards and journal](../src/chains/mod.rs), [CLI tests](../tests/cli.rs) |
| CLI integration | JSON results/errors, terminal address QR codes only for verified addresses. TUI rejects `--json`; backup remains human-only. | [CLI](../src/cli.rs), [QR](../src/qr.rs), [CLI tests](../tests/cli.rs) |
| Terminal navigation and reads | Seven tabs, a 55-action menu, one/all-network balances, session RPC selection, full receive addresses, password verification and cache repair, timed unlock and session details. Every action has a handler; a row that cannot run now (a missing prerequisite, the other chain family, a start flag) says why and starts nothing. Selecting/opening a key does not change the globally active key. | [state and tests](../src/tui/app.rs), [event loop](../src/tui/mod.rs), [terminal tests](../tests/tui.rs) |
| EVM history and transaction detail | Up to ten journal/explorer rows, pending first, explicit source coverage, hash lookup, fee/L1 fee, confirmations and registry-token receipt movements. Explorer coverage is limited to Ethereum, Optimism and Sepolia, or the Blockscout API that `ZWALLET_EXPLORER_<CHAIN>` names; each shown journal row is looked up on the node and says `checking` until then. Node/explorer content is a claim; nicknames require the session's checked list. | [history model](../src/tui/app/history.rs), [renderer](../src/tui/ui/history.rs), [UI tests](../tests/tui_history.rs), [backend tests](../tests/history.rs) |
| Terminal transfer flow | Send tab: native or non-zero registry token, checked payee, exact amount/fee review, nickname confirmation, pending/repeat overrides, journal hash and receipt deadline with typed extension. Requires a password-checked key/list and warm session. Fixed-nonce replacement controls use authenticated original intent. Real PTY acceptance and the whole local gate pass. | [transfer state](../src/tui/app/transfer.rs), [display](../src/tui/ui/transfer.rs), [21 mock tests](../tests/transfer_review.rs), [handoff](../.prd/research/handoff-tui-send-2026-10-29.md) |
| Terminal whitelist editing | Whitelist tab: local password check of the signed list, payee/contract adds, removal, fee caps, native/token/calldata per-transaction limits, message-signing toggle and reset. Each edit shows its exact summary before the password and uses the CLI's authenticated plan/apply path; an optional duration starts a new session. | [whitelist state](../src/tui/app/whitelist.rs), [display](../src/tui/ui/whitelist.rs), [terminal tests](../tests/tui_whitelist.rs), [limit tests](../tests/tui_whitelist_caps.rs) |
| Tron terminal wallet | T-address receive, TRX/USDT balances with activation/owner warnings, immutable fee review and exact nickname confirmation, expiration status and bare64 hash lookup. No Tron contract tools or replacements. Incoming transfers are not indexed. | [Tron state](../src/tui/app/transfer/tron.rs), [App/mock tests](../tests/tron_tui.rs), [lookup tests](../tests/tron_history.rs), [gate receipt](../.prd/v0-2-tui/h18-tron-tui-receipt.md) |
| TUI wallet/key creation | First-wallet form and `n` for another key, optional timed unlock, locally verified addresses and active-key update; first-wallet password confirmation, backup reminder and `B` CLI pointer. Recovery phrases never enter a TUI message. | [creation state](../src/tui/app/create.rs), [worker](../src/tui/create_runtime.rs), [receipt](../.prd/research/handoff-tui-create-ui-2026-10-29.md) |
| Terminal contract and message tools | ABI reads, checked-contract invokes with signature/raw calldata, immutable message review and typed confirmation, arbitrary-token read-only balances, and invoke speed-up/cancel/retry. | [tools API](../src/wallet/tools.rs), [tools state](../src/tui/app/tools.rs), [tool tests](../tests/tool_wallet.rs), [replacement integration](../tests/replacement_review.rs) |
| TUI coverage | `zwallet tui` has a path for every CLI function in the [coverage matrix](../.prd/03-tui.md#coverage-matrix): all 55 menu actions have handlers, and a row that does not apply to the selected chain (EVM tools or replacement on Tron, Tron rows on EVM) says so. Password-verified family addresses stay pinned for the TUI lifetime. The recovery phrase and QR codes stay in the CLI (`B` and `Q` point there). Owner acceptance on a preview build is still to come. | [action table](../src/tui/actions.rs), [state](../src/tui/app.rs), [terminal tests](../tests/tui.rs), [work units](../.prd/v0-2-tui/units.md) |

## Planned features

| Feature | Status | Evidence and current boundary |
|---|---|---|
| Solana and TON | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Architecture extension plans](../.prd/02-architecture.md). No family implementation or chain entry. |
| Cumulative spending budgets (rolling 24-hour native and per-token) | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Per-transaction limits do not track total spending; repeated transfers can still spend the full balance ([#26](https://github.com/2lab-ai/zwallet/issues/26)). |
| Calldata-aware duplicate detection for `invoke`; configurable duplicate window | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Current matching uses chain, kind, recipient and amount (plus token for token transfers), over a fixed ten-minute window. |
| User-added transfer tokens | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Reading a token by contract address is already available. |
| Secure Enclave / hardware whitelist key | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [Deferred whitelist work](../.prd/04-whitelist.md#not-in-v01-stays-planned). Today's whitelist authority derives from the wallet password; it is not a second factor. |
| Task ledger and native spend reports | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [AE-1](../.prd/07-agent-economy.md). The [example workflow](agent-workflow.md) computes a report externally from existing outputs; there is no built-in report command. |
| Investment execution adapters | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [AE-3](../.prd/07-agent-economy.md). Strategies, venue constraints, bounded approvals, typed signing and P&L accounting are not implemented. |
| Digital-service payments (including x402 exploration) | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [AE-4](../.prd/07-agent-economy.md). No built-in buyer/payment-protocol adapter or fulfillment reconciliation. |
| Merchant checkout and purchase automation | ![WIP](https://img.shields.io/badge/status-WIP-yellow) | [AE-5](../.prd/07-agent-economy.md). No shopping, order, delivery or refund integration. |

## Unsupported is not always planned

Import/restore, password changes, Windows and WalletConnect/ENS are absent without
a delivery commitment here. EIP-712 and bounded approval support are design prerequisites
for future investment/payment adapters, not implemented capabilities. The WIP table
above lists work supported by the repository's plans.

## Keeping the status accurate

When adding a feature, check its public command or UI path, its policy restrictions
and tests before moving it out of WIP. Update the README and this page together.
Keep design mockups labeled as plans until the corresponding screens can be reached.
Original requests and historical ledgers under `.prd/` remain historical evidence.

## TUI fixed-nonce recovery

TUI fixed-nonce recovery is implemented in
[the replacement boundary](../src/wallet/replacement.rs) and
[the replacement state machine](../src/tui/app/replacement.rs). Speed-up and
cancel require a password-pinned owner and an authenticated original; uncertain
retry additionally requires this session's retained review. Native, registry
ERC-20 and allowed contract-call intents retain their original nonce and kind. The
kind comes from the session's review or the node's bytes, never the journal: a
registry ERC-20 transfer recovered from the node after a restart has the same bytes
as an invoke of that token, so only its cancel is offered there.
[The focused signing and UI regressions](../tests/replacement_review.rs) count
signature requests; [the PTY acceptance cases](../tests/tui_replacement.rs) count
requests at a proxy of an isolated test daemon.
