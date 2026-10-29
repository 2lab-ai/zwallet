# Separate cohesive protocol responsibilities to reduce security review scope

Source: https://github.com/2lab-ai/zwallet/issues/28
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

src/core/protocol.rs contains about 2,324 production lines combining wire framing, process identity and lifecycle mechanics. This increases review scope for isolated changes.

Acceptance:
- Extract a cohesive responsibility into an internal module with existing API and behavior preserved.
- Keep unsafe/platform assumptions close to their implementation.
- Existing protocol and full integration tests pass; no opportunistic behavioral rewrite.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

