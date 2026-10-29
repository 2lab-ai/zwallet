# Gate preview and stable publishing on exact-commit validation

Source: https://github.com/2lab-ai/zwallet/issues/21
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

Preview publishes after preflight/build; stable release after build. Neither depends on the independent CI fmt/clippy/test job. A compilable commit with failing tests can therefore publish.

Acceptance:
- Reusable read-only validation gates run fmt, clippy -D warnings and offline tests on the exact publishing commit.
- Preview and release publication depend on that gate.
- Fork PR CI requires no secrets; stable version check remains.
- Pin the shared Rust toolchain so local and CI gate behavior agree.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

