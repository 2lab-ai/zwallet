# Multichain completion checklist
Status: implementation and local acceptance complete; post-commit delivery tracked by MC7.

Issue [#60](https://github.com/2lab-ai/zwallet/issues/60); delivery extends draft
[PR #59](https://github.com/2lab-ai/zwallet/pull/59) from ai-insightquest.

| Requirement | Delivered scope | Evidence | State |
|---|---|---|---|
| T5 | Verified Tron QR plus separate complete address in the same horizontal region | Six QR buffer decode observations, both Tron PTY sizes, unverified/impostor refusal in [evidence](evidence/README.md) | complete locally |
| T6 | Existing BIP39 seed derives Solana/TON without changing EVM/Tron | Public SLIP-0010/SDK vectors; authenticated cache migration; explicit recovery paths and TON V4R2 identity | complete locally |
| T6 | Strict native SOL/TON signing policy | Adversarial decoder tests; real-daemon exact-family policy, caps, tamper and signature/payload checks | complete locally |
| T6 | Native CLI/TUI balance, send and lookup | Loopback lifecycle; immutable key/chain review and nickname confirmation; deadline/uncertainty tests; real PTYs | complete locally |
| T7 | Plan, recovery/security docs and local gates | fmt, strict all-target Clippy, full suite: 1022 passed, 0 failed, 15 ignored; [source manifest](evidence/validation.json), links and named test references | complete locally |
| T7 | Exact-head draft PR and Linux/macOS CI | [MC7 receipt](https://github.com/2lab-ai/zwallet/issues/60) and [PR checks](https://github.com/2lab-ai/zwallet/pull/59/checks), recorded after commit | external receipt is authoritative |

Native SOL/TON scope excludes SPL/Jettons and arbitrary applications. Existing
EVM/Tron task reports are retained; no new-family task accounting is claimed.
#26/#27 remain design-only. No merge, release, owner wallet change or real-fund
send was performed. The security guide states assumptions and residual risk,
including TON's fee reserve rather than an encoded hard maximum fee.
