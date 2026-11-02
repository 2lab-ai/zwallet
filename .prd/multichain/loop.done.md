# Multichain completed implementation

Append-only observations. Contract: [`../11-multichain.md`](../11-multichain.md).

## R1 — closed 2026-11-02

| ID | Scope | Owner | Status | Evidence |
|---|---|---|---|---|
| MC1 | Same-seed derivation and native policy | key-policy | ✅ | Public SLIP-0010 vectors, exact-family tagged payees, native value/fee policy, legacy canonical-byte preservation; real-daemon policy tests. |
| MC2 | Native Solana family | solana | ✅ | Independent web3.js fixtures, strict legacy transfer decoder, signature verification, bounded RPC, genesis/fee/simulation/freshness checks and loopback CLI lifecycle. |
| MC3 | Native TON V4R2 family | ton | ✅ | Independent TON SDK fixtures, bounded/cached BOC decoder, StateInit and account identity, expiry/seqno, pacing, uncertain broadcast and outgoing-payment evidence. |
| MC4 | CLI and immutable wallet APIs | wallet | ✅ | All-family address/cache migration, recovery metadata, exact fee/preflight and wait deadlines, native send/journal/lookup, compatibility regressions. |
| MC5 | Receive QR and native TUI | tui | ✅ | Six rendered QR/address observations at 80×24/120×40; QR decode/quiet-zone/identity assertions; real Tron QR and Solana/TON send PTYs. |
| MC6 | Registry, review, documentation and local integration | root | ✅ | Eleven chains/four families, independent security review and resolved findings; full gates below. The post-commit CI facet remains MC7 in loop.md. |

Rust 1.95.0 fmt and strict all-target Clippy passed. The complete final local suite
passed: **1022 passed, 0 failed, 15 ignored**, across 42 result summaries. Source inputs did
not change during the gates; hashes, commands and test-log digest are in
[`evidence/validation.json`](evidence/validation.json). Ignored live tests are not
counted as passing. The final run follows fixes for eight integration regressions:
legacy family-address fixtures, attacker reply shapes, scoped-address error semantics,
RPC redaction fixtures and the two changed TUI interactions. Existing security
assertions remain meaningful; wrong-family errors and pay-twice warnings are preserved.

Independent review resolved TON shared-cell traversal cost, fee-estimate preflight,
TON inclusion/body-hash semantics, Solana malformed outcomes, deadline behavior and
TONCenter pacing. No confirmed source-review blocker remained. This is neither an
independent external audit nor proof against every attack. Fee/RPC/host assumptions
and unsupported capabilities are in [`../../docs/multichain.md`](../../docs/multichain.md)
and [`../../docs/security-threat-model.md`](../../docs/security-threat-model.md).

#26/#27 remain design-only. SPL/Jettons, arbitrary applications, new-family task
accounting and TON-native mnemonic import are explicitly outside this native-coin
contract. No real funds, owner wallet, new credential or live security setting was used.
Merge, release and funded acceptance remain owner actions. MC7 is closed externally
only by the exact-head delivery receipt; creating a commit alone is insufficient.
