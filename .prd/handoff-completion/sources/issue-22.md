# Honor the receipt polling deadline while an RPC request is outstanding

Source: https://github.com/2lab-ai/zwallet/issues/22
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

At src/chains/evm/rpc.rs:1443, wait_receipt checks its deadline only after fetch_receipt. A local delayed RPC reproduced a 100ms budget returning after 1.5106s. Individual requests currently have a 30s timeout, and the send lock remains held while waiting.

Acceptance:
- One deadline bounds all receipt RPC calls and sleeps.
- Deadline exhaustion reports pending/unknown, not a false transaction failure or late success.
- Regression tests cover stalled and timely replies and normal RPC failures; no live transaction needed.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

