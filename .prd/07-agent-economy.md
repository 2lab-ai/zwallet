# Agent economy: fund, delegate, account

![WIP](https://img.shields.io/badge/status-WIP-yellow) **Future product direction.** No investment engine, merchant checkout, cumulative
budget or native task-report command is implemented by this document. Reviewed
against `99fff4e` on 2026-10-29. [Current capabilities](../docs/features.md).

## Intent

The owner built zwallet because an agent doing blockchain development kept asking
for gas/stablecoins, manual transfers and manual confirmation of results. The desired
workflow is: give an agent an allocation, let it do the authorized work, then receive
an account of spending and outcomes.

The next ambition extends the same wallet component to investment agents and agents
that buy goods or services on a user's instruction. zwallet should be a replaceable,
local execution component, not a mandatory hosted agent platform, exchange, strategy
vendor or marketplace. The user chooses the agent, RPC, strategy and merchant.

## Personas and jobs

| User | Job | Desired evidence |
|---|---|---|
| Developer | Allocate funds for on-chain development work | Receipts, gas cost, failures and remaining allocation |
| Strategy operator | Authorize a bounded investment mandate | Positions, exposure, fees and realized/unrealized P&L |
| Personal-agent user | Request a purchase with an all-in ceiling | Merchant order, payment proof, delivery/refund state |
| Agent builder | Integrate one wallet boundary across workflows | Stable machine-readable intents, policy results and receipts |

## Boundary: agent / adapter / wallet

The agent proposes actions. Protocol or merchant adapters turn an approved intent
into a quote and transaction. The wallet independently checks the exact operation
it is asked to authorize. Chain/merchant outcomes are reconciled into a task ledger.
Policy changes require the human; routine actions inside a previously approved
mandate do not require a fresh confirmation each time.

Keep the signer small: no browsing, strategy logic, merchant HTTP, prices or model
calls inside the key daemon. Existing CLI/library code should be reused. Any stateful
budget enforcement needs a separate reviewed design: today's key-only daemon and
same-user-writable files cannot silently become a trustworthy cumulative ledger.

## Workstreams and acceptance

All rows are ![WIP](https://img.shields.io/badge/status-WIP-yellow). Proposed IDs are planning identifiers, not CLI commands.

| ID | Workstream | Depends on | Acceptance before removing WIP |
|---|---|---|---|
| AE-1 | Task identity and reconciled spend reports | Existing CLI JSON/journal/receipts | A task report distinguishes intent, submitted, pending, confirmed, reverted and uncertain; reconciles actual fees and balances; includes evidence and redacts personal/secret data |
| AE-2 | Cumulative task/session budgets | AE-1; reviewed authority/freshness design | Concurrent requests cannot oversubscribe; pending spends reserve capacity; replacement does not double-count; revert refunds only unspent value, never paid gas; crash/restart and rollback behavior are specified and tested |
| AE-3 | Investment execution adapters | AE-1, AE-2; bounded allowance/typed-signature design | Enforce approved assets, venues, position/exposure limits, slippage, deadlines and mandate expiry against the executable operation; stale quotes fail; losses are represented correctly |
| AE-4 | Digital-service payments | AE-1, AE-2; protocol-specific policy | Validate invoice origin, chain, asset, payee, amount, expiry and idempotency; request retries cause at most one intended payment; retain proof of access/delivery or unresolved status |
| AE-5 | Merchant checkout | AE-1, AE-2, adapter protocol | Bind product/options/merchant to a quote with tax, delivery and conversion costs; do not equate on-chain payment with a completed order; handle pending orders, cancellations and refunds |
| AE-6 | Agent integrations | AE-1 and supported adapters | Reference shell integration first; optional framework/MCP tools expose no wallet password or policy-edit authority by default; the same scenario passes across adapters |

### AE-1: report first

Start with the existing JSON outputs and journal; retain the distinction between a
local record and chain truth. Reports contain task ID, intent, chain, asset, amount
in exact units, recipient/contract, tx hash, nonce/replacement relationship, actual
fees, outcome and remaining allocation. Native gas, tokens and fiat valuations are
separate. Cost-basis and price timestamps are mandatory for P&L; receipts alone do
not establish investment profit. The included report example is not AE-1 complete.

### AE-2: budget authority

Decide the enforcement boundary before implementation. Native/token transfers,
indirect contract effects and signature-based authorizations require different
accounting. Define reservation, settlement, failed execution, reorg/finality policy,
multiple keys/processes and persistence. Do not advertise rollback-proof limits
using only state an agent's OS user can restore or modify. Per-transaction limits
already exist and remain useful; they are not cumulative budgets.

### AE-3: investment is a mandate, not a promise

A user may authorize an agent to pursue returns within an explicit mandate. zwallet
must not promise profits, supply undisclosed advice, or imply principal protection.
The first adapter should be one narrowly defined action on an approved venue, with
simulation and receipt-based accounting before broader strategies.

Today's revoke-only token approvals and lack of EIP-712 signing intentionally block
many DeFi workflows. Do not relax them globally to make a demo work. Bounded approvals
and typed signing require independent policy review, domain/chain/spender checks and
specific tests. Pause/cancel cannot undo a mined trade or guarantee a stop-loss price.
Strategy selection and model reasoning belong to the agent, outside the signer.

### AE-4 / AE-5: purchase is more than a transfer

Investigate an x402 buyer adapter using the open protocol, without committing to one
facilitator or assuming existing EIP-191 signing authorizes payment safely. Digital
access needs evidence of fulfillment. Physical goods also need merchant identity,
product/options, tax, shipping, conversion quote expiry, refunds and dispute state.
Shipping/payment-account personal data stays outside the signer and public logs.
A native-token payment receipt does not prove delivery or a refund right.

## Proposed milestones

1. **Development allocation:** reproducible current CLI workflow and script-level
   report (available in `examples/agent-budget`; no new core feature claim).
2. **Accountable tasks:** AE-1, then the AE-2 authority design and enforcement tests.
3. **One paid-service adapter:** AE-4 with a controlled service, duplicate-payment
   tests and receipt/fulfillment reconciliation.
4. **Investment and shopping pilots:** separate AE-3/AE-5 scopes after their policy
   prerequisites. Use simulations/test environments before any owner-funded pilot.

No date commitment. Prioritize by actual user workflows and avoid adding a broad
trading/commerce surface before reliable accounting and policy boundaries exist.

## Demonstration and launch rules

- Current demo: real binary, local Anvil, valueless ETH, scripted orchestration,
  selected JSON fields. Preserve raw cast and execution evidence.
- Future scenes: persistent **CONCEPT / WIP — NOT AVAILABLE** label. No invented CLI
  commands presented as shipping, merchant confirmations or executed trades.
- Do not imply automatic top-ups, fiat/card checkout, guaranteed income, gasless
  stablecoin use, per-agent sandboxing, a security audit or universal token support.
- Publish evidence and reproducible examples. Native task reports and the broader
  workflows remain WIP until acceptance passes.

## Research

The [dated comparison](../docs/marketing/research.md) covers TWAK, WDK, Coinbase
Agentic Wallet CLI, smart-account approaches and x402. Existing alternatives matter:
our claim is a particular local operating model, not that agent wallets do not exist.
