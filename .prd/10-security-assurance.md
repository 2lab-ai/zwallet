# Security assurance and remaining-plan completion

Status: in-progress delivery, 2026-10-30. Authorized implementation and local acceptance
are complete in this proposed branch; final CI/merge state is linked from
[the completion checklist](security-assurance/completion-checklist.md). The implemented
contracts below describe current source behavior, not a future target.

## Goal and boundary

Finish authorized remaining work from the repository's actual plan and explain the
security of the implemented wallet with falsifiable code and test evidence. Reconcile
merged work before creating anything: main 0994f9a contains PR #58; PR #59 remains an
open draft and is continued on its existing ai-insightquest fork branch.

The security document must not promise resistance to every attack. For each attack
class it records attacker capabilities, implemented defenses, exact implementation
and regression evidence, assumptions, and residual risk. Planned work is not a defense.
The original local custody, no-daemon-file-write and no-external-store choices remain;
the explicit #26/#27 design-only decision has not changed. Funding, real-wallet access,
new credentials, security-policy expansion, releases and PR approval are not inferred.

## Acceptance

| ID | Required observation |
|---|---|
| S1 | A scope matrix maps current plan/SSOT rows to already delivered, remaining implementable work, and exact owner/architecture dependencies; completed TUI/Tron work is not duplicated. |
| S2 | `docs/security-threat-model.md` covers key storage/signing, backup/recovery, spending rules, transaction substitution/replay, chain/RPC compromise, dependencies/builds, host compromise, secret output, concurrency, permissions, phishing and DoS. Every defense links code and named tests or is explicitly marked unverified. |
| S3 | The existing TUI resize and post-exit-output test notes have direct observations if still absent; ordinary builds expose no test-only fault mechanism. |
| S4 | The existing Tron fee-field and TransferActuator documentation notes are resolved against actual code and primary upstream evidence; no live funded send is substituted. |
| S5 | Relevant local tests, the repository fmt/Clippy/full gate, documentation links/loop lint and final PR CI pass on the submitted source. The final report lists concrete residual blockers separately from completed authorized work. |
| S6 | AE-1 has a native read/report path with task identity; intent/submitted/pending/confirmed/reverted/uncertain remain distinct, amounts and fees use exact per-asset units, balances and replacement identity carry provenance, and personal/secret data are redacted. Local records never become chain truth or budget authority. |
| S7 | `invoke` duplicate matching includes canonical calldata, and a backward-compatible authenticated duplicate-window setting is editable by the human. The existing default and signatures for absent fields remain compatible. The guard remains explicitly advisory and does not claim daemon replay or cumulative-budget enforcement. |

## Activated remaining scope

The latest request to finish planned development activates AE-1 and the two duplicate
guard items already written in `07-agent-economy.md` and `04-whitelist.md` (an explicit
interpretation of the broad continuation, not a claim that they were original TUI/Tron
handoff obligations). Implementation reuses the CLI, journal, receipt and signed policy
boundaries. It adds no signer authority, external credential, real transaction, hosted
service, arbitrary-token support or new persistent trusted counter.

AE-2 remains design-only under the owner's #26/#27 decision. AE-3/4/5 depend on AE-2
and separate protocol policy; they cannot be represented as shipped by bypassing that
dependency. AE-6's shell reference follows AE-1; unselected framework/MCP adapters,
hardware custody and new chain families remain future designs. Owner-funded acceptance
and stable release gates remain owner actions.

## Execution

### AE-1 command and trust contract

`task create <id>` binds one chain and account, captures exact native/registry balances
and optional declared allocations in integer base units. `transfer` and `invoke`
accept optional task attribution, record intent before signing and preserve uncertainty
before broadcast. `task report <id>` reconciles existing read APIs; offline mode makes
only local claims. Default output redacts addresses; explicit reveal is available.
No arbitrary descriptions, raw calldata, signed bytes or credential-bearing RPC URLs
are stored as task metadata. Task files are local advisory data, never authenticated
budget authority. Missing fees, receipts, bounded-journal gaps and unresolved replacements
prevent definite remaining-allocation claims. Balance deltas include unrelated flows
and are not automatically assigned to a task. EVM and Tron must both be covered.

The TUI offers actual task create/list/report actions in Tools and immutable optional
task attribution in transfer/invoke reviews. Reads use asynchronous generation-tagged
effects, default redaction and explicit provenance/time/unknowns. These are not new
exceptions to the existing CLI/TUI coverage requirement.

### Duplicate guard compatibility contract

Optional signed global `policy.duplicateWindowSecs` accepts 1 through 86400 seconds.
Absence, reset and `default` retain the existing 600 seconds; zero is not a disable
setting. Human `whitelist duplicate-window <duration|default>` and its TUI action use
the authenticated edit boundary. The optional field is omitted from legacy canonical
signing bytes. Invoke identity includes keccak256 of normalized encoded calldata;
legacy/malformed missing identities match conservatively instead of bypassing the guard.
Only the existing bounded 1 MiB journal tail is searched. CLI local-file settings are
labeled unverified; TUI uses its password-checked review. Actual signing independently
verifies policy. Overrides, direct daemon calls, file deletion or other homes can bypass
this advisory repeat warning; it is not an anti-replay or cumulative-budget boundary.

Reuse current implementations and receipts. Add only the code/tests needed by the
scope audit and observed gaps. Root owns repository documentation; evidence auditors
do not edit those files. One heavy process/CLI test suite runs at a time, with isolated
test homes and the existing daemon scope. Preserve original commit attribution and
add `Co-authored-by: Z <z@2lab.ai>` to each new commit.
