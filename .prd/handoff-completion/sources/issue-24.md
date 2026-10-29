# Keep daemon expiry responsive when wallet/socket filesystem checks stall

Source: https://github.com/2lab-ai/zwallet/issues/24
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

Shared::check reads wallet.json and stats d.sock synchronously under the session mutex on a current-thread Tokio runtime. O_NONBLOCK does not make regular-file or path metadata I/O nonblocking. A stalled filesystem can delay automatic expiry/key release, unlike the already bounded whitelist reader. This was a static finding; the external lock process still has a forced-termination fallback.

Acceptance:
- Move wallet and socket inspection off the event-loop and outside the session lock.
- Bound worker concurrency and waits; workers hold no seed.
- Fail closed on inspection timeout; expiry/lock remain responsive and late results cannot sign.
- Regression tests inject stalled inspections and retain replacement checks.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

