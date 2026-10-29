# Joint policy rollback — decision and future contract

Status: planned, 2026-10-29. The design decision is recorded; strong joint-rollback
protection is unimplemented. This is not a shipped security feature.

Intent: prevent a previously revoked permission from returning after local files are restored.
Action: retain the documented limitation and define the external authority needed to change it.
Decision: do not claim freshness from another same-user file or add a new trust service.

## Source and current boundary

Issue: [#27](https://github.com/2lab-ai/zwallet/issues/27). The owner's decision on
2026-10-29, preserved verbatim:

> 기존 원칙 유지: 한계를 명시하고 설계 이슈로 추적

The current sealed wallet authenticates `{whitelistPub, whitelistGen, whitelistDigest}`
and the policy-key salt. The signed list authenticates its canonical policy contents.
Together they prove that the pair matches. They do not prove that no newer pair existed.
Restoring an older matching `wallet.json` and `whitelist.json` can restore revoked payees,
selectors or less restrictive caps. The password may still open an authentic older wallet;
signing a generation counter inside that wallet does not make it fresh.

The current daemon pins the ciphertext it unlocked and exits when a changed ciphertext is
observed; reads and final liveness checks also prevent a delayed read from reviving that
session. This protects that live session's observation, not the choice of wallet opened by a
later unlock. Joint rollback after the previous daemon has ended remains accepted. The
baseline regression is `w5b_an_older_wallet_and_list_restored_together_are_accepted_and_shown`
in `tests/evm_offline.rs`; its acceptance describes a limitation, not a desired future fix.

Current recovery remains the current `wallet.json` if an older one was put back, otherwise
`whitelist reset` and deliberate re-entry of permissions. A reset is a new policy grant; it
does not prove historical freshness. Never suggest an older list as a rollback remedy. An
operator checking the shown generation and entries can notice a discrepancy, but memory,
screenshots, CLI output and a backup alongside the wallet are not a security authority.

## Why the present constraints cannot provide the guarantee

Compare two executions: one stops legitimately at policy P, and one advances to Q, revokes
an entry, stops, and then restores every locally observable file to P. With no state outside
that rollback set and no retained trusted process, a new daemon receives the same inputs in
both executions. It cannot accept the legitimate P while rejecting the restored P.

A hash chain, timestamp, signed counter, separate database or file outside `ZWALLET_HOME`
does not change this if the same agent can replace it with an older authentic copy. Filesystem
locks order cooperating writers; they do not stop same-user restoration. A signature proves
who authorized a policy, not which authorized policy is newest. No implementation under the
present no-external-trust/no-daemon-file-write rules is labelled a strong rollback fix.

## Requirements for a future trust authority

The following is a future interface contract, not an available API, selected vendor or
deployment recommendation. Any choice changes the present architecture and needs a separate
review and authorization. A persistent budget also needs its own ledger from [08](08-cumulative-budget-design.md).

An authority must be outside the attacker's ability to erase, fork, restore or impersonate it.
It must bind all copies of one signing authority to one durable record and atomically compare
and advance that record. Merely moving storage out of the home is insufficient. Candidate
boundaries are:

| Candidate | Evidence required before it can be selected | Cost or unresolved boundary |
|---|---|---|
| Protected local broker under a different OS principal | Enforced access control, authenticated client/broker identity, atomic durable state, non-resettable identity binding, fencing of old sessions and crash tests | Changes the same-user-only custody model; root and machine rollback remain explicit threats. |
| Hardware-backed authority | A proved anti-rollback state primitive, authenticated reads/updates, clone/reset/loss behavior and a signing path that cannot bypass it | “Hardware”, “Secure Enclave” or a protected key alone is not evidence of a usable monotonic store. No OS keychain is introduced. |
| Remote authenticated authority | Linearizable updates, anti-replay freshness, stable identity binding, privacy and offline-denial behavior, compromise/recovery model | Adds a service/network dependency; the current daemon makes no RPC and has no network login. |
| On-chain policy wallet | Every relevant spending path is governed by the policy, with explicit finality/reorg and recovery rules | Changes the custody/address/execution model; does not retrofit freshness into ordinary local EOA signatures. |

No candidate is approved or implemented by this document. Loss or denial of the chosen
authority must fail closed for protected signing; silently reverting to local-only mode
would restore the original vulnerability.

## Minimum state and transition contract

1. **Stable identity.** Bind enrollment to the signing authority, not home path, key name,
   active key or a newly invented local UUID. The authority must recognize copies of an
   enrolled seed/key and refuse unprivileged re-enrollment as a fresh wallet. A seed imported
   into another signer outside this boundary can still spend; that limit must be stated.
2. **Commitment.** Keep a monotonic authority revision and a domain-separated commitment to
   the current wallet cryptographic envelope and policy anchor, including the policy-key
   identity, generation and canonical digest. This distinguishes two different policies at
   the same local generation and older password/KDF envelopes with unchanged policy.
   Epoch changes after reset are explicit transitions under the same identity, not new
   histories. Do not expose the mnemonic, password or policy-signing secret to the record.
3. **Authenticated read.** Unlock must compare the successfully decrypted wallet/list pair
   with the latest authoritative commitment using a replay-resistant read. A stale, absent,
   ambiguous, unreachable or mismatched record cannot yield protected signing. A record
   response cached in a same-user file is not a latest-state proof.
4. **Edit transaction.** Authenticate the human's exact proposed change and bind it to the
   expected authority revision. Prepare and validate the new sealed wallet/list bytes, then
   stop or fence every old protected session before committing the transition. Advance by
   atomic compare-and-swap; only one concurrent editor can commit a successor. Publication
   of local files can finish afterward, but a failed publication never moves the authority
   backward. A second editor must reread and show the actual current change before approval.
5. **Signing fence.** All protected signers, including copies in other homes/hosts, must obey
   the same revocation fence. A check only at unlock is insufficient: a still-running old
   daemon would keep its old grant. The design must order authorization and policy commits
   at one authority, or co-locate the protected signer and authority so an old session cannot
   release a newly authorized signature after revocation commits. A CLI-reported revision or
   revocable token cached indefinitely is insufficient. Signatures already exposed before
   revocation remain valid; policy freshness cannot recall them from clients or the chain.
6. **Crash recovery.** Define one durable commit point and test every transition boundary.
   Before it, the authoritative predecessor is still current; after it, only the committed
   successor is current. If local publishing is incomplete, accept only bytes matching that
   latest commitment, whether available as a staged candidate or through authenticated
   recovery. Never fall back to an older matching pair, decrement the revision, or silently
   treat an absent record as first use. Conflicting or missing recovery material blocks
   signing until the explicit recovery path completes.

The authority's record format, enrollment proof, fencing protocol, atomic storage guarantees
and transport remain implementation prerequisites. This contract deliberately does not claim
that a counter by itself makes the two local files and all live signers an atomic system.

## Resets, password changes and recovery

| Event | Required future behavior |
|---|---|
| `whitelist reset` | Advance the existing authority revision and commit the newly approved empty/default policy. Do not reset the authority's history or detach the wallet identity. Old epochs remain stale. |
| Password or KDF change | Re-encrypt the wallet and re-sign under the newly derived policy key as required, then commit the successor envelope and anchor under the same authority identity. Both old password/envelope and mixed old/new pairs must fail protected unlock. |
| Restart, home relocation or restoration | Revalidate against the authority; no “first run” exemption based on directory contents. Concurrent copies share the same revision and fencing scope. |
| Lost or corrupted local files | Authenticate recovery and obtain/reconstruct the latest committed material or authorize a new successor. The phrase recovers keys but does not attest the latest policy. Stale or missing records fail closed. |
| Authority outage | Refuse protected signing with a bounded error; do not silently disable protection or use a cached revision as fresh. |
| Lost/reset hardware or authority state | Explicit recovery by a separately authenticated authority outside the attacker-controlled state. If that authority cannot prove continuity, disclose that the old guarantee is lost; require a new protected identity/custody migration rather than claiming an uninterrupted history. |

Recovery authority, backups and identity migration must be specified before choosing a
backend. Resetting an external record merely because the requester can supply a valid old
encrypted wallet would make the attack a recovery feature. Password-based recovery alone
does not distinguish an old valid password and wallet from the latest authorized state.

## Future acceptance gates

These are required scenarios for a future implementation, not claims of present protection.
Run with isolated test keys and a controllable authority; no real wallet or funds are needed.

| ID | Exercise | Required observation |
|---|---|---|
| F1 | Restore both matching old files after a revoke, restart and unlock | Zero signatures under the revoked policy; mismatch reports freshness failure. Keep current W5b as the explicitly unprotected-mode baseline. |
| F2 | Restore old pairs across first enrollment, reset and policy-key rotation | No downgrade to unprotected first use; old epochs remain rejected under the same identity. |
| F3 | Change password/KDF, then supply old envelope/list and old valid password | Protected unlock refuses; a mixed pair also refuses. A legitimate complete new pair works. |
| F4 | Two policies share a local generation; race two authorized edits | Digest binding rejects the wrong policy; one atomic successor wins; the loser cannot overwrite it. |
| F5 | Crash before/after preparation, fence, authority commit, each local publish and cleanup | Only the defined current state signs; ambiguous/missing material fails closed, with no authority rollback. |
| F6 | Keep an old daemon alive in a renamed/cloned home or another host while revoking | Old sessions cannot authorize after the fence's commit point; previously released signatures are explicitly outside recall. |
| F7 | Delete all local counters, clone home, replace UUID, replay recorded authority responses | Identity and fresh-read checks prevent re-enrollment, rollback and stale response acceptance. |
| F8 | Authority missing, reset, partitioned, timed out or impersonated; local clock changed | Bounded fail-closed outcome and no local fallback; errors disclose no secrets. |
| F9 | Recovery with latest material, stale material, phrase only and lost authority | Only the documented authenticated path restores protected service; any lost guarantee is explicit. |
| F10 | Re-run unsigned/unknown schema, file tamper, single-file rollback and current policy tests | Existing fail-closed behavior remains; freshness work does not bypass decoded-byte policy or secret handling. |

Keep #27 open as a deferred security design issue until an authority is approved, implemented
and these tests produce observed receipts. This design fulfills the requested documentation
work; it does not change current joint-rollback behavior or justify a “fixed” issue closure.
