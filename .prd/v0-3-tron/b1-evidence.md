# Tron B1 — strict codec evidence

Status: implemented and locally verified (2026-10-29; exact-head CI and upstream merge pending).

This unit adds only the bounded transaction codec and cryptographic primitives over
Tron A. The daemon policy and public send command remain later B2/C boundaries.

The decoder accepts one canonical TransferContract or TriggerSmartContract. It rejects
unknown, duplicate and reordered fields at every nesting level, nonminimal/overflow
varints, wrong wire tags, extra contracts, bad Any type URLs, invalid 21-byte address
prefixes, invalid positive int64 amounts and inputs over 1 KiB before nested allocation.
Re-encoding must match the bytes. Exact ABI transfer parsing rejects trailing words
and dirty address padding. Memo, permission and unsupported protobuf fields are refused.

New signatures are deterministic low-S, 65 bytes with recovery id 0/1, over SHA256
of raw_data. Historical fixtures cover high-S and legacy 27/28 recovery. The signer
primitive is separate from the future daemon policy; parsing is not permission to sign.

Fourteen focused tests cover node-built golden vectors, real historical signatures,
all truncations and malformed-byte mutations; fixture provenance is retained in
`tests/fixtures/tron-{unsigned,signed}.json` and the numbered Tron spec.
Independent codec review found no remaining blocker. No transaction is broadcast by
this unit. The parent records the full gate and exact-head CI in the PR before readiness.

Full Rust 1.95.0 gate on the integrated B1 tree passed: fmt --check, all-target locked
Clippy with warnings denied, and cargo test --locked: 557 passed, 0 failed, 13 explicitly ignored live tests.
The x86_64-apple-darwin library test build also passed; it was not executed on Intel.
Gate log: /tmp/zwallet-b1-full-gate.log (local); Intel log: /tmp/zwallet-b1-intel.log.
