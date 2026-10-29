# H19 — TUI9 fixed-nonce replacements

Status: in-progress — backend and UI implemented; backend independent review clear.
UI independent review is CLEAR after the fixes below; parent serial CLI/PTY gates are pending.

## Contract and provenance

The contract is `.prd/03-tui.md` §Status of a send / Replacements and
`.prd/v0-2-tui/units.md` TUI-9, T27–T29. The parent tracks H19 in the handoff loop.
The implementation baseline is `fa79ca7` plus the stable send and history backend patches;
the retained-transfer snapshot API is a dependency owned by the send UI unit. Baseline files
were copied to `/tmp/zwallet-h19-baseline` before H19 edits; its dependency snapshot is updated
when another unit's stable code is adopted. The H19 patch excludes those dependency changes.

`src/chains/evm/tx_original.rs` reconstructs supported legacy EIP-155 and type-2 EIP-1559
transactions from required node fields. It computes the EIP-2718 byte hash and recovers the
signer, requiring the requested hash, pinned own address, chain and nonce. Node `from` is only
a consistency check. Unsupported types, nonempty access lists, mined records, missing fields,
high-s signatures and inconsistent fields are refused without reflecting node text.
Encoding references: [EIP-155](https://eips.ethereum.org/EIPS/eip-155) and
[EIP-1559](https://eips.ethereum.org/EIPS/eip-1559).

`wallet::replacement` accepts an immutable in-session review or a proven pending transaction
matched to the journal's chain, owner, hash, nonce, kind, recipient, base-unit amount and token.
The journal alone cannot construct an original. Retry is only available from a retained
session review, including when the node does not know the hash. Every replacement retains
the original nonce and hash; cancel is zero native value to the pinned own address.
Invoke kind and calldata remain intact, with a current checked contract and selector required.

The original's authenticated fee fields provide the replacement floor; the normal RPC fee
estimates, ceilings, nonce-consumed check, funds checks and OP L1 estimate still run. Confirm
reprepares under the send lock, verifies the current whitelist fingerprint, and returns a new
review without signing if intent, policy, funds or the integer total-fee maximum changed.
Cancellation carries the original-race warning. No raw signed transaction escapes the API.

## Observed validation, 2026-10-29

- RED: the valid-original proof test failed against the initial refusal-only implementation.
- GREEN: `cargo test --lib chains::evm::tx::original::tests`: 2 passed.
- GREEN: `cargo test --test replacement_review`: 17 passed, including all signer counters.
  These tests use temporary `/tmp/zwt-*` homes, a fake Unix-socket signer and loopback RPC;
  they start no real daemon and access no wallet or external RPC. The sandbox denied local
  socket creation on the first attempt; the same bounded tests passed after approved escalation.
- GREEN: `cargo clippy --lib --test replacement_review --test tui_replacement -- -D warnings`.
- Target directory: `target-handoff-tui-unlock`, assigned idle by the parent.
- Full CLI/PTY/all-target gates are owned by the parent and have not been run by H19.

T27: native/token reconstruction, restarted and retained-session speed-up, changed journal
amount/destination/nonce/kind/token, forged signer/changed bytes, missing address pin, fee cap,
execution/OP L1 fee increase, consumed nonce. T28: exact `cancel`, same nonce, pinned self,
zero value/empty input, cap/increase refusals. T29: retained retry with unknown hash despite
modified/old or absent journal; restart cannot retry; unknown node original cannot authorize
speed-up/cancel. An additional invoke test preserves kind/calldata and requires the checked
contract selector.

The eight UI mock/TestBackend tests additionally cover proof-before-affordance,
nonce/hash/typed-confirmation binding, replayed Enter, wrong nickname, forged
proofs, a detail closed while its proof is pending, current fee re-review,
queued L followed by ReviewChanged, same-session uncertain retry after journal
corruption, and the exact restart warning with an old or missing journal.
Independent review found truncated long quantities/omitted invoke calldata and a
cancel retry that unnecessarily required a self-address whitelist entry. Both
failed regression tests before their fixes, then passed. A further regression
confirmed that an uncertain cancel must show both its race and duplicate-payment
warnings; all six warning rows now fit the minimum 80×24 frame, including the Wait
form. A mined receipt removes R and the uncertainty warning, and direct R key
input dispatches no preparation effect. That direct-dispatch regression also
failed before the guard was added.
Every frame checks bounded inert outside text, expected styles and zero replaced
cells. `cargo test --test tui_replacement --no-run` compiles two process tests:
T27/T28 restart proof plus speed-up/cancel, and T29 uncertain retry plus restart
with aged/deleted journal. The PTY cases use the isolated real daemon without relocating its socket and
verify signed broadcast envelopes; the mock tests above count every SignTx.

## UI boundary and integration

History schedules a separate read-only original proof after an exact scoped
pending-detail result. Only its still-current request can adopt the proof and
enable s/x. The signing review pins chain/RPC, verified key/index/address, checked
list fingerprint, hash, nonce and action. Stale preparation replies cannot reopen
a closed review. Same-session transfer snapshots are retained only after the
accepted progress message associated with a locally computed, journaled hash;
invoke snapshots use the tools unit's opaque RetainedInvoke map. Retry never
constructs intent from a local journal. Unknown details show the warning before
other transaction fields, including when no journal row remains.

Confirmed work uses the common non-cancellable send/status flow. An L queued
while confirm reprepares is forwarded as LockAll even if reprepare returns a new
review; current locking state prevents confirmation of that new review. The
parent's newer receipt deadline and terminal mutation-drain changes are separate
dependencies and must remain in the merged transfer/event-loop code.

Required standalone dependencies: H13 send UI + RetainedTransfer + verified_key,
H17 EVM history/detail (optional journal nonce), and the shared deferred-lock and
queued-quit-error fixes. The main H19 delta compiles without the Tools module.
Its baseline is `/tmp/zwallet-h19-standalone-base`; its complete source snapshot
is `/tmp/zwallet-h19-standalone`. The two typed RetainedInvoke adapters are a
separate `/tmp/zwallet-h19-h15-invoke-hooks.patch` for after H15. The full worktree
includes H15 only as a validated integration snapshot; those implementations do
not enter the standalone H19 patch. The parent owns the newer receipt deadline,
terminal drain and later Tron optional-status-nonce adaptations.


Final independent UI review: reviewer reran all eight UI focused tests after the
last key-dispatch fix, all GREEN; no open blocker. Local report with reviewed
source hashes: `/tmp/zwallet-h19-independent-review.txt`. The shared quit-error
fix was also independently cleared in `/tmp/zwallet-send-quit-error-independent-review.txt`.

## Standalone PR integration, 2026-10-29

The reviewable integration tree is `.worktrees/handoff-tui-replacement-pr`, branch
`handoff/tui-replacement-pr`: H13 `685a363` plus the uncommitted H17 merge
`9addb25`, followed by the standalone H19 delta. The H17 merge resolutions are
staged separately from H19; no H19 commit or merge commit was made by this unit.
The merged dependency snapshot is `/tmp/zwallet-h19-pr-merged-baseline`.
H13's receipt deadline post-check, terminal mutation drain and queued-quit error
preservation remain intact. H17's explorer test server read/write timeouts remain
intact. The shared deferred-lock `return_external_review -> Vec<Effect>` fix is
included without any Tools type or module dependency.

Observed in this integration tree: strict all-target Clippy and format checks
passed; replacement mock tests 17, original-transaction crypto tests 2, quit-error
tests 2, terminal-failure tests 5 and history TestBackend tests 10 passed. An
inherited history test initially expected the old `s/x/R unavailable` placeholder;
it now asserts the key-verification hint, no signing affordances, and no effects
from any of s/x/R. Both PTY tests compile; no CLI/PTY process suite was executed.
The parent owns those final gates and the merge/feature commits.

The H15 adapter remains `/tmp/zwallet-h19-h15-invoke-hooks.patch`. Existing invoke
coverage proves the backend's recovered intent and current-selector checks; the
complete Tools UI → retained Progress → s/x/R path with signer counters is still
pending after H15 integration and must not be represented as completed T34(g–l).

## Parent runtime corrections, 2026-10-29

The first whole gate exposed a real shortcut-focus regression: typing `1x` into
the wait-duration field dispatched the replacement cancel shortcut. Idle s/x/R
now dispatch only on the status screen with no transfer input field. The existing
end-to-end wait/extension test failed before this guard and passed afterwards;
it also asserts that the wait form retains focus and replacement remains idle.
An independent source review cleared the narrow guard.

The first real PTY run exposed a fixture mistake: relocating d.sock correctly
causes the production daemon to terminate under its socket-integrity guard.
The fixture now leaves the actual daemon socket untouched and verifies recovered
signer, fixed nonce, destination/value/calldata and local broadcast counts.
Exact zero/one SignTx assertions remain in the independent mock suite; the PTY
receipt does not claim wire instrumentation of the real daemon. A stale retry
hint expectation was aligned with the reviewed UI text.

Observed actual PTY acceptance: T27/T28 speed-up and zero-self cancel passed;
T29 uncertain same-session retry and restart with aged/deleted journal passed.
Only temporary wallets and a loopback node were used. The complete fmt/Clippy/
full-suite gate is repeated after these corrections before publication.

Final parent gate: Rust 1.95.0 fmt, locked all-target strict Clippy and full tests
passed: **650 passed, 0 failed, 12 explicit live-test skips**. Both real PTY
cases passed again within this whole gate. Log: `/tmp/zwallet-h19-final-full-gate.log`.
