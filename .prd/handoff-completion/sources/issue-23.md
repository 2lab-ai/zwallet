# Restore the required Clippy gate and pin the tested toolchain

Source: https://github.com/2lab-ai/zwallet/issues/23
Observed state on 2026-10-29: closed (completed)

Review baseline: `9ecfe932b0feb6427290125091e959e281135328` (2026-10-29).

Rust 1.95.0 reports clippy::collapsible_match at src/tui/app.rs:208. The repository gate uses -D warnings, so it fails. Existing 492 tests and fmt passed during review; this is not a claim about the current remote CI outcome.

Acceptance: simplify the nested match/if, pin the tested Rust toolchain, and run fmt, all-target Clippy and the full non-live suite.

Implementation tracking: `.prd/06-review-hardening.md` and `.prd/review-hardening/`. Delivery is a PR from ai-insightquest's fork; no merge or deployment is authorized.

