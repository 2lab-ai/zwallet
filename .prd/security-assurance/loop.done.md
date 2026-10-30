# Security assurance — completed implementation

Append-only observations. Final source inputs and local counts are in
[`evidence/local-validation.json`](evidence/local-validation.json). Post-commit
GitHub conclusions are maintained on [PR #59](https://github.com/2lab-ai/zwallet/pull/59)
and the [issue #57 delivery receipt](https://github.com/2lab-ai/zwallet/issues/57).

## R1 — closed 2026-10-30 (authorized implementation)

The remaining implemented plan is complete in this proposed branch. No new stable
release, merge, owner-wallet acceptance or funded transaction is claimed.

### Closed rows

| ID | Scope | Owner | Status | Evidence |
|---|---|---|---|---|
| S1 | Actual approved-plan reconciliation | plan auditor / root | ✅ | Main 0994f9a includes marketing PR #58; existing #59 reused; the completion checklist maps all approved/retained/dependent scope. |
| S2 | Threat model and attack/evidence matrix | root, custody/signing reviewers | ✅ | `docs/security-threat-model.md`, source/named tests, assumptions, residual risks and unsupported claims; independent factual review; links and test names checked. |
| S3 | TUI resize and complete post-exit observations | terminal unit / root | ✅ | Actual SIGWINCH shrink/grow with refresh ticks disabled, exact shared PTY EOF tail and panic tests; final complete suite below. Original TUI notes moved append-only to its R3. |
| S4 | Tron fee fields and contract-recipient rule | signing reviewer / root | ✅ | `05-tron.md` branch values and official pinned java-tron condition; later reference block qualified; original Tron notes moved append-only to its R2. |
| S6 | AE-1 native task identity and reconciled reports | report/TUI units / root | ✅ | CLI and actual TUI create/list/report plus immutable transfer/invoke/replacement attribution, both chain families, six states, exact accounting/provenance/redaction. Tests and four PTY scenarios in evidence. |
| S7 | Calldata duplicate identity and authenticated window | duplicate unit / root | ✅ | Optional signed backward-compatible window, canonical calldata identity, CLI/TUI editing and source labels, conservative legacy matching; advisory scope retained and regression-tested. |

### Measured gates and review

Rust 1.95.0 fmt and strict all-target Clippy passed. Complete suite: **964 passed, 0 failed, 15 ignored**.
Focused integration regressions: 89 passed. Complete suite was rerun after all
integration fixes. [Source manifest and terminal frames](evidence/README.md) bind
these observations to the submitted inputs. No test used an owner's wallet or real
funds. Live ignored tests are not silently counted as passing.

Independent custody, signing/report and terminal reviews were clear after corrections.
The full gate caught and corrected integration-only issues: a provenance substring
false positive, an action-table merge, a missing menu hint, outdated calldata duplicate
expectations, retry warning overlap, and missing combined repeat-warning/provenance
text. Executed final evidence, not the earlier unit reports alone, supports closure.

### External delivery state

S5 remains an external post-commit gate in `loop.md`: the exact final commit and both
CI platforms must be checked before reporting delivery complete. Its final conclusion
is recorded in the linked PR/issue receipt, avoiding a self-referential source commit.
The input manifest makes code identity checkable independently of documentation edits.
#26/#27 stay design-only; dependent adapters, funded TR23, owner preview acceptance,
QR/phrase trade-offs and release/merge authority remain as listed in the scope matrix.
