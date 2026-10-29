# Add signed per-transaction amount and calldata limits

Source: https://github.com/2lab-ai/zwallet/issues/25
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

The documented policy allows the entire balance to listed payees and any native value/arguments to allowed contract selectors. Per-transaction execution fee caps exclude OP-stack L1 data fees.

Acceptance for the incremental fix:
- Optional authenticated limits constrain native value, registry-token transfer amounts and calldata size using the daemon's decoded transaction.
- Human CLI can set and inspect them with explicit units; absent limits preserve existing signed-list verification.
- Bounds and malformed policy tests; no CLI-only enforcement.
- Clearly state that calldata length is only exposure mitigation, not an L1 total-fee cap, and arbitrary contract argument effects remain outside generic selector policy.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

