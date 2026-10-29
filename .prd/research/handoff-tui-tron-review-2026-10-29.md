# H18A — immutable Tron transfer review backend

Scope: TUI12 / T40, TRX and registry USDT only. T38/T39/T41 screens and status wiring remain
with the parent/UI work unit. No funded send, public broadcast or PTY claim is made here.

## Boundary

`wallet::tron_send` offers `tron_transfer_payees`, `review_tron_transfer` and
`send_tron_transfer`. `TronCheckedPayee` has private fields and comes only from the retained
password-checked list's address entries whose chain scope is exactly `["tron"]`. Wildcard,
mixed-chain, contract and zero-body entries are excluded. `TronTransferInput` has an asset enum
(TRX/USDT), amount, held key name/index/password-pinned T address, payee and duplicate override.
There is no nonce, replacement, arbitrary token, pending override or gas-price input.

The immutable review keeps the checked list, fingerprint, endpoint, key, decoded transaction,
policy verdict and integer bounds together. The caller must supply its lifetime password-derived
key/index/address pins, and must refuse metadata index changes before starting a review. A daemon
status response does not establish those pins. Confirm accepts only the consumed review, the
exact nickname typed by the human, and a progress callback; it cannot replace a reviewed input.

Both review and confirm check the fingerprint before and after RPC. Confirm takes the account
send lock and prepares against the held recipient. Fresh reference bytes, timestamp and expiration
are allowed, but owner, contract, native amount or exact token calldata, key and asset are held.
`max_fee_sun` and `max_cost_sun` are compared as U256 values calculated before display formatting;
a rise returns `ReviewChanged` with no SignTx. A drop can proceed. Insufficient funds or invalid
preflight fails before signing. The checked in-memory list decides the policy verdict.

`tron::send::prepare_review` preserves integer estimated/max cost, max fee and native balance;
existing CLI `prepare_transfer` simply returns its `prepared` transaction. The maximum keeps C's
conservative resource-race/account-activation behavior. The UI can show all existing `fees` fields,
`trxLeftSun`, and getters for the exact integer bounds; left TRX is an "at least, at prices of
block N" amount, not a promise about later chain parameters.

The common send tail preserves exact prepared rawData equality, strict canonical signature and
recovered owner checks, local txID/decoded intent, case-sensitive T-address equality, fsynced
expiration journal before progress/broadcast, and no nonce in Tron journal/output. It never
prints; the CLI adapter supplies its existing stderr callback.

## Observed validation

- RED: `fee_rise_never_signs_and_returns_a_new_review` returned a mock `Sent` when the integer
  maximum comparison was absent. GREEN after adding both integer fee and total-cost bounds.
- `cargo test --test tron_review`: eight passed. Includes TRX/USDT rising/falling fees, advanced
  expiration with identical payment, fingerprint changes before/during review and confirm,
  foreign chain/bad sender/amount before RPC, exact nickname, checked policy cap, send lock,
  duplicate refusal/held override, fund shortage, signer raw substitution/wrong key/truncated
  signature, zero broadcast after invalid signer replies, and fsynced journal before callback.
- Existing `tron_send` mock cases: 18 passed; parent-owned real daemon/CLI case explicitly
  skipped. Existing `tron_offline`: 13 passed. Only temporary homes and loopback mock endpoints.
- Strict all-target Clippy passed. Formatting and final focused library result are recorded in
  the parent handoff receipt. The parent owns the complete commit gate and UI/PTY integration.

## Integration notes

Source dependency is Tron C `deda723d4f6575b23f0084715b619f5b40f9be8d`. The isolated worktree
contains a minimal CheckedList/check_list dependency and C's tail extracted into wallet/send.rs
so focused tests run independently; do not overwrite the integrated root wallet.rs/send.rs with
those dependency copies. Root already owns the fuller TUI2/TUI5 APIs.

Apply the H18 backend patch (new module/tests, integer-cost preparation, minimal module export),
then the separate root shared-send integration patch. The latter keeps the existing EVM exact
payload verifier and adds only the C family branches. If root's `PreparedTx` already has
`max_gas_cost`/`l1_fee`, set the Tron constructor to `max_gas_cost: costs.max_fee, l1_fee: None`.
Retain the root SendRecord progress callback plus C's omission of nonce when `signed.nonce` is
None. Do not restore C's old printing SendRecord implementation. No commit is made by this unit.
