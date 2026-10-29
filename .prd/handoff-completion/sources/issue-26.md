# Design cumulative spending budgets without weakening the custody boundary

Source: https://github.com/2lab-ai/zwallet/issues/26

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

The existing fee cap is per signature; repeated valid transactions can each spend it. Strong cumulative budgets need authoritative state, concurrency/replacement/dry-run accounting and restart/rollback semantics.

User decision on 2026-10-29: preserve same-user/local-only custody and the no-daemon-file-write/no-external-store principles; track this as a design issue rather than pretend mutable CLI state enforces a security boundary.

Open design acceptance:
- Define session versus persistent budgets, reservations, retries and shared-key accounting.
- Demonstrate resistance to same-user counter edits/restarts or explicitly document the weaker guarantee.
- No external trusted storage/service introduced by the current fix PR.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.
