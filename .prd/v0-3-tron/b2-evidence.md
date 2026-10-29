# Tron B2 — signed policy evidence

Status: implemented and locally verified, 2026-10-29; exact-head CI and upstream merge pending.

B2 depends on B1's strict decode refusal list in `b1-evidence.md`. Its reviewed boundary
checks the daemon-derived owner, bounded expiration, usable authenticated list, exact
Tron-only payee scope, supported direct transfer and signed amount/calldata/fee limits.
No RPC or daemon file writes are introduced. The protocol still carries only unsigned
raw_data, never a client-provided recipient/asset/clock policy claim.

Tron payees are stored as a 20-byte body with exactly `["tron"]`; legacy EVM wildcard
canonicalization remains unchanged and `*` never authorizes Tron. A first edit/reset
now adds the 30-TRX default signed fee_limit cap. Historical *Wei field names store sun
on Tron. Bandwidth and activation charges remain explicitly outside fee_limit.

The malformed-byte corpus passes through the real daemon request handler and records
zero signature attempts; valid TRX/USDT controls record two. Focused whitelist CLI
checks passed 2/2 in parent integration, and independent codec/policy review found
no remaining blocker. Local whole-repository and remote exact-head CI results are
recorded before readiness. Public transfer/lookup remain C, not this unit.

Integrated Rust 1.95.0 gate: fmt --check, locked all-target Clippy with warnings denied,
and full cargo test --locked passed: 570 tests passed, 0 failed, 13 explicit live skips.
The first attempt reused stale B1 package metadata in a shared Cargo target and failed
to compile two tests against an old payee signature. After cleaning only zwallet package
artifacts, the complete gate above passed against B2 source. No test was disabled.
Local log: /tmp/zwallet-b2-full-gate.log. No external network or funded transfer was used.
