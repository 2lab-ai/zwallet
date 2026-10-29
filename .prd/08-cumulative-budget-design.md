# Cumulative spending budgets — decision and future contract

Status: planned, 2026-10-29. The design decision is recorded; cumulative enforcement is
unimplemented. This is not a shipped security feature.

Intent: bound what an unlocked agent can authorize without moving custody to mutable CLI state.
Action: retain the existing stateless limits and specify the boundary a future budget needs.
Decision: no cumulative budget is added under the current local-only architecture.

## Source and decision

Issue: [#26](https://github.com/2lab-ai/zwallet/issues/26). The owner's decision on
2026-10-29, preserved verbatim:

> 기존 원칙 유지: 한계를 명시하고 설계 이슈로 추적

The inherited constraints are same-OS-user clients, no external trusted storage or service,
no OS keychain, and no daemon file writes. The daemon derives and signs; it does not ask RPC
whether a transaction was sent. The CLI, journal, wallet-home files and client-provided
timestamps are not authorities for expenditure or available budget.

The current signed `transactionLimits` and fee caps remain **per transaction**. Splitting a
payment into many valid requests can still spend the whole balance. No counter, daily reset,
new policy field or command is introduced by this document. The open implementation remains
tracked in #26; completing this design does not mean that repeated spending is prevented.

## Three distinct guarantees

| Scope | Meaning | Boundary and decision |
|---|---|---|
| One live daemon session | Upper bound on distinct authorizations issued by that process after one unlock | A future in-memory counter can enforce this while that session survives; no continuity across unlocks, cloned homes, independent daemons or other signers. Not implemented. |
| Persistent authorization budget | Upper bound on authorizations across sessions and copies of the same signing authority | Requires an authoritative, atomic ledger outside the agent's rollback authority, or custody in a signer that owns such state. Deferred under the current constraints. |
| Spending in a chain-time window | Upper bound on value actually executed in a given interval | Issuance time is insufficient. Transactions can be withheld and executed later; EVM transactions here have no general expiry. Requires an on-chain enforcement policy or separately proved validity and trusted settlement rules. Deferred. |

A session budget must be named **session authorization budget**, not a daily spending limit.
A persistent rolling 24-hour issuance limit must explicitly say that signatures from different
windows may execute together. The conservative first future design below is a non-refilling
authorization allowance. A human explicitly grants a new allowance; old signatures remain
valid, and their exposure must not disappear merely because the allowance was renewed.

Restoring all local files and starting a new session yields the same local inputs as an older
legitimate state. An encrypted or signed counter detects edits but accepts authentic older
copies. A journal hash chain, lock file, local database, second wallet-home file or renamed
home cannot distinguish those histories. Killing a daemon does not reveal the password or
automatically unlock again; it does erase any counter held only by that daemon. Human
re-unlock is therefore a new budget grant unless a stronger authority persists the old debt.

## Accounting contract for a future implementation

The following is a specification for review, not an available API or policy schema.

**Identity and units.** A ledger scope binds a stable wallet authority, its policy epoch and
all derived key indices covered by that grant. Key names, the active marker, home paths,
process IDs and addresses supplied by the CLI cannot create new allowances. Repeated copies
of the same signing key share its scope; independently authorized signers must share the
same ledger for a cross-signer claim. A wallet-wide budget sums subkeys; an explicit per-key
budget is a different product and must say so. Amounts are checked integers in smallest units,
keyed by chain identity and native asset or canonical token contract. Never sum raw units of
different assets, infer cross-chain equivalence, or use RPC exchange rates as policy authority.

**Exposure.** Compute the vector from the daemon's canonical decoded signing bytes, after
ordinary whitelist and per-transaction checks. Count native value, including self-transfers,
and direct supported token transfers separately. Reserve the decoded maximum execution fee
in a native fee bucket as well; if a total native allowance is claimed, native value plus fee
must also fit that total using checked arithmetic. This remains no claim about OP-stack L1
fees or any other charge the signed payload does not bound. Unsupported or unbounded effects
fail closed in a mode advertised as a complete spending budget. Arbitrary contract calls and
message signatures can delegate spending not described by those amounts; disable them in
that mode unless a separately reviewed decoder supplies a sound bound. A narrower direct-
transfer budget must prominently disclose these exclusions instead.

**Reservation state machine.** Let `limit[a]`, `charged[a]` and `reserved[a]` be the allowance,
irreversible authorization charges and in-flight reservations for asset `a`.

1. Decode and validate the exact payload and its signer, chain and policy epoch. Obtain a
   domain-separated request identity from those fields and bytes, never from a caller ID.
2. Under the ledger's atomic admission operation, require
   `charged[a] + reserved[a] + exposure[a] <= limit[a]` for every bucket, without overflow.
   Reserve the entire vector or nothing. A token transfer that exhausts the fee allowance
   cannot partially reserve its token allowance. Concurrent requests see the reservation.
3. Recheck session liveness, current policy and reservation ownership before signing. A
   reservation that is known never to have produced any signature may be released.
4. Charge before exposing a signature to any client. For durable mode, the authority must
   durably commit before signature bytes cross its trusted boundary. A crash or uncertain
   handoff between signing and commit must retain the full reservation as charged exposure;
   loss of availability is preferable to returning that allowance twice.
5. Delivery failure, client disconnect, timeout, journal deletion, “dry-run”, RPC rejection,
   unknown receipt, apparent drop or daemon restart never refunds a potentially released
   signature. A session implementation retains this conservative charge until that session
   ends; it cannot claim persistence after the in-memory state is gone.

The current successful `--dry-run` obtains a real signature. It must consume allowance under
this future contract. A future unsigned preview can avoid that only by never invoking the
signer. RPC responses or journal edits cannot prove that a signed payload is unspendable.

**Retries and replacements.** Rebroadcasting identical signed bytes issues no new signature.
An exact duplicate request may reuse a committed charge only for identical canonical bytes,
signer, chain and policy epoch, with an authoritative record that survives every claimed
reset boundary. A missing record is not evidence of prior payment; refuse or charge again.
Do not return cached signatures from a revoked epoch, expired/locked session or invalid list.
The initial design charges distinct replacement/cancellation payloads independently, even
when their nonces match. A same-nonce optimisation requires a separate proof about chain
identity, mutually exclusive execution, reorgs and fee exposure; an untrusted CLI's “replace”
label is not such a proof. Retries rebuilt with a new nonce or Tron reference block/expiry are
new authorizations. Reaching a claimed expiry is not automatically a refund rule.

**Reset, policy edits and time.** Lowering a budget below existing exposure blocks new
authorizations; it does not erase debt. Raising, resetting, changing password, renaming a key,
switching networks or migrating a home must preserve charges for the scope a persistent
mode claims. A fresh independent grant must disclose unresolved old signatures. Use monotonic
elapsed time for live-session scheduling; neither OS wall-clock changes nor RPC timestamps
may refill a security allowance. Trusted time alone still cannot make old EVM signatures
expire. The freshness authority in [09](09-policy-freshness-design.md) is necessary for a
persistent ledger's rollback resistance but does not by itself supply atomic budget charging.

## Future acceptance gates

All rows below are required design tests for a future feature; none is a claim of present
test coverage. Use test keys, isolated homes and mock authorities. Do not broadcast funds.

| ID | Exercise | Required observation |
|---|---|---|
| B1 | Two concurrent requests each fit alone but exceed a shared allowance together | At most one distinct authorization escapes; all asset reservations update atomically. |
| B2 | Native/token/fee edges, zero, huge values and checked-add/multiply overflow | Exact edge succeeds; excess and overflow sign nothing; units never cross assets or chains. |
| B3 | Allowed signed dry-run, then live send; disconnect after signing; ambiguous RPC failure | Potentially exposed signatures remain charged. Unsigned preview requests no signature. |
| B4 | Exact request replay, altered nonce, altered fees, cancellation, altered Tron expiry | Exact replay follows the proved idempotency rule; every changed payload reserves independently. |
| B5 | Rename key, change active key, use another covered index, copy/rename home | No new budget within the advertised authority scope; session-only mode discloses its limits. |
| B6 | Kill at every reservation/sign/commit/delivery boundary; restore all mutable files; restart | Persistent mode never releases an unpaid authorization; unknown outcomes retain exposure. Session mode makes no continuity claim. |
| B7 | Roll OS clock back/forward, spoof RPC time/receipts, delete journal, withhold signed bytes | No untrusted input refills allowance or proves non-execution; delayed execution limitation is visible. |
| B8 | Reset whitelist, change password, lower budget, revoke an entry with a cached signature | Debt is preserved where promised; no cached signature is returned under revoked policy. Previously disclosed bytes cannot be revoked by this ledger. |
| B9 | Contract-mediated transfer, delegated message signature, unknown asset or unbounded fee | Complete-budget mode refuses before signing; narrower mode accurately labels excluded effects. |
| B10 | Duplicate signing key in two homes/hosts; cloned or unavailable authority | Persistent mode uses one atomic scope and fails closed without it; no local fallback. |

Before implementation, choose and separately approve either the explicitly weaker session
feature or a changed custody/state boundary. Keep #26 open as deferred design tracking until
that choice and these gates are implemented. A future PR must record exact threat coverage,
reset semantics and the observed tests, rather than close the issue with a mutable counter.
