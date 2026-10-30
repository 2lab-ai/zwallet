# Give an agent an allocation, then collect the receipts

[README](../README.md) · [Commands](commands.md) · [Security](security.md)

This guide is for a developer delegating on-chain work to an agent that can run shell
commands. zwallet supplies a wallet, policy enforcement and transaction evidence.
Native [task reports](task-reports.md) collect accounting evidence; your agent supplies
the task logic and final interpretation. No particular model, agent
framework or wallet-service account is required.

## Human setup

1. Use a dedicated wallet home and fund it only with the allocation for the task.
   Keys under one home share a seed and signing policy; names are not isolation.
   Stronger isolation requires a separate OS user/container and an appropriate setup.
2. Create the wallet, back up the recovery phrase privately, and verify the funding
   address. A stablecoin balance does not pay native gas automatically.
3. Add the exact permitted payees/contracts and chains. Configure fee and amount
   limits. For example, `whitelist native-cap sepolia 0.005` caps one native transfer,
   not the sum of a session's transfers. Contract argument effects are not bounded
   simply by capping native value or calldata length.
4. Unlock for the task window. Give the agent the wallet home and the task, not your
   password or recovery phrase. Use `--password-stdin` only with a controlled input
   source; do not store the password in an environment variable or a file.

Follow the [quick start](../README.md#quick-start) for exact commands.

## Example task instruction

The following is a prompt for your own agent, not a built-in zwallet feature:

> Use zwallet to complete the approved development payment on Sepolia. The human has
> funded the wallet and allowed @alice. First inspect the balance and dry-run a
> 0.001 ETH transfer. If the policy allows it and funds cover the payment and gas,
> send once and verify the receipt. Do not request or handle my password, change the
> whitelist, exceed the task allocation, or obtain additional funds. If the result
> is pending/unknown, look up the existing hash; never send another payment merely
> because a command timed out. Report the purpose, recipient, amount, actual gas,
> hash, receipt status and remaining balance. Escalate policy changes or unresolved
> failures; routine permitted transactions do not need another confirmation.

The prompt's total-spend instruction is agent behavior, not an enforced cumulative
budget. The daemon enforces the configured signing policy on transaction bytes.

## Execution and reporting contract

| Step | Current mechanism | What to preserve |
|---|---|---|
| Inspect | `balance --chain … --json` | Chain, asset, exact base units, remaining gas balance |
| Start attribution | `task create <id> --chain … --allocation native=<base units> --json` | Bounded nonpersonal task ID, advisory allocation and baseline observation |
| Preview | `transfer … --dry-run --json` | Intended payee/value, policy result, funds check; this can sign but never broadcasts |
| Execute | `transfer … --task <id> --wait --json` or a permitted `invoke --task <id>` | `txHash`, task/intent identity, nonce, journal path and outcome |
| Reconcile | `tx --chain … <hash> --json` | Receipt success/revert/pending/unknown; a journal entry alone is not success |
| Report | `task report <id> --json` (or `--offline` for local claims) | Six outcome states, exact fees/amounts, balance provenance, unknowns and replacement relationship; agent adds purpose outside the wallet |

For accurate EVM gas expense, a report can read the receipt over the same RPC and
multiply `gasUsed` by `effectiveGasPrice`. On chains with extra fees, account for
those separately; this formula alone is not an all-chain total-cost calculator.
The included local demo uses a plain local EVM chain with no L1 surcharge.
Token amounts and native gas must be separate columns; never add unlike assets or
invent a USD conversion without a price source and timestamp.

A reverted transaction still consumes gas. A pending transaction remains pending in
the report. Never label a timeout, an unsigned preview, a journal write or a submitted
hash as a successful payment. See [retry safety](commands.md#send-journal-and-retry-safety).

## Reproducible example

[Run the recording script](../examples/agent-budget/README.md). It creates a disposable
local-chain wallet and executes the actual binary. It checks the recipient balance,
receipt, unchanged nonce after policy refusal and the exact balance equation:

`initial allocation = confirmed payment + actual gas + remaining balance`

The committed recording's report remains the historical script result; its assets
have not been regenerated to imply a new execution. For current shell integration,
use [native task reports](task-reports.md). Cumulative budgets and investment/merchant
adapters remain ![WIP](https://img.shields.io/badge/status-WIP-yellow) in the
[agent-economy PRD](../.prd/07-agent-economy.md).
