# Document the trusted-state requirement for joint policy rollback protection

Source: https://github.com/2lab-ai/zwallet/issues/27

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

Restoring a previously valid wallet.json and matching whitelist.json resurrects old permissions. Their signatures authenticate content, not freshness. This is already a documented limitation, not a newly discovered signature bypass.

User decision on 2026-10-29: retain the current local-only/no-external-trust architecture and keep this as a design issue.

Acceptance for a future security feature:
- Specify a trusted freshness anchor outside attacker-controlled state and recovery behavior.
- Prove rollback detection across resets, password changes and restarts.
- Do not describe another same-user writable file as rollback protection. Current PR must retain the limitation and avoid claiming a fix.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.
