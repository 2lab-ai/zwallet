# Task identity and spend reports

[Commands](commands.md) · [Agent workflow](agent-workflow.md) · [Security boundaries](security-threat-model.md)

Use a task to attribute development payments and collect their observed outcomes.
A task binds one chain and one wallet account. Its allocations are accounting labels,
**not enforced spending limits**: local task files and send journals are writable by
the same OS user, and untagged or externally signed payments are not discovered.

## Shell workflow

With a wallet and authorized session already prepared by the human, create an ID
containing no personal data or secrets. IDs contain 1–32 lowercase letters, digits,
underscores or hyphens, starting with a letter or digit.

```sh
zwallet task create dev-payment --chain sepolia --key main \
  --allocation native=20000000000000000 --json
zwallet task list --json
```

The create operation reads baseline native/registry-token balances and writes only
local task metadata. Allocation values are exact integer base units, not decimal coin
amounts. Repeat `--allocation TOKEN_CONTRACT=BASE_UNITS` for a registry token. No
password or signing is needed to create, list or report a task; the cached account
binding alone is not a verified funding address.

For a separately authorized payment, add `--task dev-payment` to the existing
`transfer` or `invoke` command. All existing signing-policy, pending, duplicate and
confirmation rules remain. For example, on an isolated test setup:

```sh
zwallet transfer --chain sepolia --key main --to @dev-service --amount 0.001 \
  --task dev-payment --wait --json
zwallet task report dev-payment --json
zwallet task report dev-payment --offline --json
```

These examples do not provision funds or authorize a payment. The task records a
shaped intent before the signing request, then records uncertainty before broadcast.
A successful broadcast updates the local submission claim. Interruption or an update
failure must not turn an attempted send into a claim that no payment occurred. A
dry-run can sign but does not broadcast through this path; its task record remains
an intent without a recorded submission.

`task report` uses the task's recorded chain and the configured RPC, or `--rpc URL`.
It performs a bounded observation pass, without signing, broadcasting or rewriting
task history. `--offline` makes no RPC requests. EVM and Tron use their existing
transaction lookup paths; a node's response remains a claim, not a consensus proof.

## Read the states and amounts

| State | Meaning | What it does not prove |
|---|---|---|
| `intent` | A local prepared intent exists, without recorded submission | No signature ever existed, or no outside payment happened |
| `submitted` | The local send path recorded a successful broadcast response | Inclusion, execution success or finality |
| `pending` | The observed node reports a matching pending transaction | Eventual inclusion or execution |
| `confirmed` | Matching transaction evidence and the node's successful mined result | Irreversible finality, exact token economics, delivery or profit |
| `reverted` | Matching transaction evidence and the node's failed execution result | Zero expense: failed execution can still consume fees |
| `uncertain` | Evidence is missing, inconsistent, unknown, failed or too late | Safe to retry as a new payment |

Reports keep native gas, token amounts and declared allocations separate. Integer
`*Raw` fields use each asset's base units; `null` means unavailable, never zero. A
confirmed-intent total describes the intended direct value, not a guarantee about
arbitrary contract effects or token issuer behavior. Actual fees require receipt
evidence, including extra chain fees where applicable. Unknown fee components keep
accounting incomplete.

Balance changes include **all** account activity between observations, not only this
task. Baseline and current observations are separate reads, not an atomic snapshot.
Remaining allocation is provided only when the tracked accounting is sufficiently
complete; it still is not an authorization reserve or a promise that the account
cannot overspend. Missing/deleted task records and untagged actions are outside it.
No fiat conversion, cost basis or investment P&L is inferred.

EVM attempts carry nonce and replacement relationships. Conflicting observations
that report distinct hashes mined at the same nonce must not be summed as definite
payments. A cancellation races the original; it cannot undo an already mined payment.
Tron has no nonce replacement. Expired or unseen transactions remain unsafe to repeat
without further investigation.

## TUI and privacy

In Tools (`6`), `a` opens the task list, `n` creates a task and Enter opens its report.
The action menu also exposes task create/list/report. In a report, `r` refreshes from
RPC, `o` selects local-only observations, and `a` toggles address reveal. Create uses the open key
and selected chain and confirms exact allocations before reading the baseline. The
report view supports offline/refresh and explicit address reveal; source, observation
time, warnings and unknown fields remain visible. Transfer and invoke reviews carry
optional task attribution (`t`) into execution rather than re-reading a mutable
selection at signing time. In-session replacements inherit the retained task; a
transaction reconstructed from history starts unassigned until explicitly selected.
A task persistence warning stays visible on the send status even after its receipt
arrives; receipt success does not repair a failed local metadata update.

Addresses are hidden in ordinary task output; `task report --reveal-addresses` includes
them deliberately. Redaction is not anonymity: task IDs, transaction hashes and hashed
address labels can remain linkable. Registry asset addresses are public identifiers.
Do not put personal information, passwords or seed material into task IDs or public
transaction data. Raw calldata, usable signed transactions and credential-bearing
RPC URLs are not task descriptions.

Each task is bounded to 256 intents and a wallet to 256 task records; metadata is
size- and shape-checked. This prevents unbounded in-memory history expansion through
the supported file format, not arbitrary host denial of service. Local files can be
deleted, restored or replaced; this report is not a protected ledger or an audit log.
TUI metadata jobs and the asynchronous create/report/send metadata workers have a
bounded wait, but a timed-out write may finish later. The CLI's initial wallet/task
lookup and task-list directory reads have no overall wall-time guarantee against a
stalled filesystem. Worker wait limits do not establish a whole-process exit deadline
for an already-running blocking syscall. Inspect the task list/report before retrying. A successful broadcast with an unconfirmed local
update carries a warning; it must not be interpreted as a failed payment.
