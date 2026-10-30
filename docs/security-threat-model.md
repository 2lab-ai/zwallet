# Security threat model and attack evidence

[Custody details](security.md) · [Feature status](features.md) · [Commands](commands.md)

zwallet reduces specific risks of giving an agent a locally unlocked wallet. It does
**not** make a wallet safe against every attack. Its strongest application boundary
is that the daemon decodes the exact unsigned transaction and checks the authenticated
policy before signing it. That boundary assumes an authentic running program, a
trustworthy host and a password/seed that the attacker has not obtained.

This is a source-backed security explanation, not an independent audit, a penetration
test certificate or a proof of cryptographic correctness. Review date: 2026-10-30.
Named tests below are regression evidence for the specified paths; their presence is
not a claim that every adversarial interleaving or operating system was tested. The
[completion record](../.prd/security-assurance/completion-checklist.md) identifies the submitted
revision and actual validation. Planned features never count as current defenses.

## Assets, attackers and trust boundaries

The protected assets are the recovery phrase and derived private keys, the human's
password, policy-edit authority, authorized transaction bytes, and reliable knowledge
of which address and operation the human is approving. Availability and accounting
integrity matter too, but local files and RPC responses are not trusted authorities.

| Actor / capability | Intended boundary | What remains possible |
|---|---|---|
| Prompt-injected or malicious agent, same OS user, no password/seed | Request signatures only within the loaded authenticated policy; cannot mint a valid policy edit using only the daemon's intended session state | Read/write local metadata and journal, call every permitted operation repeatedly, consume fees, deny service, exploit shared-user binary/terminal/process access |
| Different unprivileged OS user | Unix directory/file/socket permissions and peer-UID checks | Privileged access, permissive ACLs, owner mistakes and OS bugs are outside this boundary |
| Malicious RPC, explorer, token or allowed contract | Untrusted input parsing, exact-intent checks, strict signing decoder/policy, explicit provenance | False balances/fees/receipts, withheld broadcasts, reorgs, harmful allowed contract effects and misleading identity claims |
| Stolen encrypted wallet file | Password-based authenticated encryption and bounded memory-hard derivation | Offline password guessing, deletion, restoration of older authentic files |
| Password, phrase, root/kernel, installed binary or trusted terminal compromised | No claim of prevention | Password permits decryption and policy edits; phrase permits signing outside zwallet; privileged host can observe or replace the execution |
| Dependency, build runner or release account compromised | Version/commit pinning, review, validation and limited workflow permissions reduce exposure | Malicious pinned code or a compromised publisher can still ship malicious binaries |

The one-shot CLI performs RPC and transaction construction. The daemon keeps the seed
and public policy anchor in memory, derives keys and signs, reads policy for checks,
and makes no RPC calls or wallet/policy/journal state writes. Its lifecycle still
creates the daemon lock and creates, chmods and removes its socket. The agent and CLI are outside the
signing-policy trust boundary. The daemon still shares the operating-system user;
it is not a hardware wallet or an agent sandbox.

## Key generation, storage and password attacks

| Attack | Implemented defense and code | Regression evidence | Assumption / residual risk |
|---|---|---|---|
| Predictable or malformed phrase | OS randomness and BIP39 validation in [`Seed::generate`](../src/core/seed.rs) | `generated_seeds_have_requested_word_count_and_are_distinct`; `invalid_phrase_error_does_not_echo_words` | Trust OS entropy and crypto dependencies. Two distinct test samples do not prove entropy quality. No BIP39 passphrase is supported. |
| Copy `wallet.json` and guess its password | AES-256-GCM; fresh salt/nonce; scrypt default N=2^18, r=8, p=1; password minimum 12 characters in [`store`](../src/core/store.rs) | `encrypt_decrypt_round_trip`; `wrong_password_is_wrong_password_error`; `create_master_writes_only_wallet_json_with_strict_modes` | A length floor is not password strength. An attacker can guess offline without an online rate limit. No time-to-crack estimate is warranted. |
| Modify ciphertext, sealed policy or parameters during an edit | GCM authentication and envelope/snapshot checks in `store::open`, `MetaLock::reseal` | `tampered_file_fails_closed`; `sealed_policy_survives_metadata_rewrites`; `reseal_refuses_an_envelope_changed_on_disk_after_open` | Authentication does not establish freshness; a complete old authentic pair can be restored. Checks are not an atomic filesystem attestation against arbitrary hostile writers. |
| Inflate KDF cost or substitute a FIFO, symlink or huge file | KDF bounds checked before derivation; 32-byte salt; regular owner-owned no-follow, nonblocking, bounded 1 MiB wallet read | `kdf_policy_bounds_memory_before_any_allocation`; `decrypt_rejects_kdf_params_outside_policy`; `wallet_file_reads_refuse_fifos_links_and_oversized_files` | The scrypt parameter uses 256 MiB by default; whole-process RSS is not bounded to that amount. Many processes can each run a KDF. Some filesystem metadata calls can still stall. |
| Creation race/crash destroys an existing wallet | Exclusive writer lock and temporary file, fsync, decrypt/read-back, hard-link publication without replacement in `store::create_steps` | `second_master_is_wallet_exists_and_file_untouched`; `failure_before_publication_removes_only_created_paths`; `published_wallet_is_never_unlinked_on_a_later_failure` | Does not stop same-user deletion, disk failure or loss of all copies. Filesystem durability semantics remain trusted. |
| Read another user's files or connect to their socket | Created home 0700, files/socket 0600; client/server peer-UID checks in [`protocol`](../src/core/protocol.rs) and [`daemon`](../src/daemon/mod.rs) | `create_master_writes_only_wallet_json_with_strict_modes`; peer checks inspected in `send_answered` and `serve` | Cross-UID enforcement was inspected, not proven by a cross-UID exploit test here. Same UID and root are not isolated. Existing file modes are not universally repaired. |

## Unlock, memory and local host compromise

| Attack | Implemented defense and code | Regression evidence | Assumption / residual risk |
|---|---|---|---|
| Fake public daemon steals the unlock password | Private child socketpair handshake; password never sent over `d.sock`, argv or environment in [`spawn_daemon` / `drive_daemon`](../src/core/protocol.rs) | [`c1_fake_daemon_on_the_socket_never_receives_the_password`](../tests/cli.rs); `r1_unlock_handshake_over_a_socketpair`; Linux `a_pipe_reopens_through_proc_but_the_socket_channel_does_not` | Protects this transport path, not a malicious parent process or stolen password source. |
| Replace executable while human types | Linux starts `/proc/self/exe`; macOS checks file identity and started child's matching cdhash/CS_VALID before password delivery | `k5_binary_swapped_during_the_prompt_never_gets_the_password`; macOS `a_started_daemon_that_runs_other_code_is_given_nothing` | Replacing the CLI before launch defeats this. On macOS an interposed program can fork a holder of the startup channel then exec the genuine image. Matching code identity does not prove exclusive channel ownership or publisher identity. |
| Leak inherited descriptors or inject child environment | Child environment cleared; descriptors marked close-on-exec including descriptors above the soft limit | `k12_daemon_inherits_no_stray_descriptor`; `r3_daemon_inherits_no_descriptor_above_the_soft_limit` | Does not cleanse code or environment already affecting the parent CLI. |
| Debugger/core-dump/memory extraction | [`disable_core_dumps`](../src/core/password.rs) requests RLIMIT_CORE=0 and Linux non-dumpable status. Linux unlock checks Yama and process visibility before prompting | `ptrace_scope_check_reads_the_given_path`; Linux `r1_unlock_refuses_ptrace_scope_0_without_the_flag` | Hardening syscalls are best effort: return values are not checked. Existing tracer, allowed ancestor, root/kernel, swap, hibernation, physical memory and side channels are not prevented. No `mlock` guarantee. |
| Recover secrets after normal drop or forced kill | Principal password and phrase owners use `Zeroizing`; TUI avoids cloned/debuggable password state in [`password`](../src/tui/password.rs) | `paste_wipes_its_copy_and_input_has_one_bound`; `matching_moves_one_owner_and_failure_and_cancel_empty_both_buffers`; [`create_accepts_a_non_clone_password_owner_and_drops_it_before_returning`](../tests/create_wallet.rs) | Not every copy is wiped. `Seed` documents unwiped dependency intermediates; backup creates ordinary string/JSON copies. SIGKILL does not execute destructors. No full-memory erasure proof exists. |
| Wrong re-unlock destroys current session / concurrent unlock creates extra sessions | New child decrypts before old session stops; `u.lock`, lifetime `d.lock`, cleanup on failed start | `k6_wrong_password_on_re_unlock_leaves_the_session_running`; `c3_concurrent_unlocks_leave_exactly_one_daemon`; `c6_squatter_on_the_daemon_lock_fails_unlock_and_leaves_no_daemon` | Cooperative serialization plus process cleanup is not immutable against hostile same-user writers or process creation. Holding locks can deny service. |
| Parent dies during startup, leaving a hidden unlocked child | Bounded go/readiness handshake; failed readiness clears the session and exits | `r8_daemon_wipes_and_exits_when_its_cli_dies_after_the_go_byte` | A successfully reported session deliberately persists after CLI exit. Startup decryption is outside steady-state I/O worker deadlines. |
| Agent uses the unlocked session without separate consent | Authenticated policy bounds every decoded signature | [`signed_limits_apply_to_the_daemons_payload_and_can_be_removed`](../tests/policy_limits.rs) | There is no per-agent identity, isolation or human confirmation per permitted action. Every process with the same user can request permitted signing during the session. |

The [`zeroize` guarantees](https://docs.rs/zeroize/1.9.1/zeroize/#what-guarantees-does-this-crate-provide)
apply to the memory actually zeroized, not all copies a program or dependency may
have made. A stolen seed bypasses the whitelist completely: the attacker can sign
with another program. Keeping policy-edit authority out of the daemon does not
make seed extraction harmless.

## Recovery, backups and address phishing

| Attack / failure | Implemented defense and code | Regression evidence | Assumption / residual risk |
|---|---|---|---|
| Silently redirect a recovery phrase to a file | [`backup`](../src/commands.rs) requires terminal stdin and stdout, then a fresh password check | [`backup_refuses_without_a_terminal`; `c14_backup_refuses_a_terminal_on_only_one_side`](../tests/cli.rs) | A PTY meets this check; it is not proof of human presence. Recording, scrollback, clipboard, remote terminal and phishing can capture an intentionally displayed phrase. |
| Lose wallet state and expect phrase to restore everything | Standard, tested EVM/Tron derivation paths; explicit backup reminder | [`known_vectors_index_0_and_1`](../src/chains/evm/mod.rs); [`tr1_the_a1_mnemonic_gives_tronwebs_key_0_and_the_next_index`](../src/chains/tron/mod.rs) | Phrase preserves keys with matching paths and no passphrase, not names, active key, policy, journal or task records. No native import/restore/password-change command exists. Third-party recovery for every wallet/index has not been tested here. |
| Replace cached receive address or run a fake listener | Local password-derived address verification; session code/process checks; unverified cache labeling | [`k1_squatter_answering_the_cached_address_is_never_verified`; `s7_own_code_behind_a_symlink_to_zwallet_with_its_uuid_is_not_verified`](../tests/cli.rs) | Genuine code on a fully substituted attacker wallet is a documented residual case: `k1_swapped_wallet_with_a_genuine_attacker_daemon_is_the_named_residual_risk`. Verify with the known password and compare a previously trusted address. |
| Phishing nickname, clipboard swap or permitted wrong recipient | Checked list entries and exact typed review confirmation; QR only from verified address paths | [`same_generation_resigned_payee_change_signs_nothing`](../tests/transfer_review.rs); [`wrong_confirmation_policy_denial_lock_and_duplicate_all_refuse_before_signing`](../tests/tron_review.rs) | The software cannot determine the real-world identity or honesty of an address. A human can authorize the attacker's address. A compromised terminal can draw false content outside zwallet. |
| Restore older backup and re-enable revoked policy | Anchor binds key, generation and digest; mismatched list/anchor fails closed | [`whitelist` tests](../src/core/whitelist.rs); [policy-freshness design](../.prd/09-policy-freshness-design.md) | A jointly restored old `wallet.json` and matching list passes. No external freshness authority exists. Preserve current backups separately; reset does not prove an older pair cannot return. |

Losing both usable recovery material and the password/current encrypted wallet can
lose access permanently. Publishing a phrase is equivalent to publishing spending
authority over all its derived accounts; deleting the published copy cannot revoke
copies already taken. Recovery and policy freshness are separate problems.

## Spending and signing policy

| Attack | Implemented defense and code | Regression evidence | Assumption / residual risk |
|---|---|---|---|
| Forge or edit a payee/selector/cap without the password | Ed25519 signed whitelist pinned by sealed public key, generation and digest; separate domain-derived policy key in [`whitelist`](../src/core/whitelist.rs) / [`store`](../src/core/store.rs) | `the_key_is_derived_from_the_password_and_a_sealed_salt_and_never_written`; `an_edit_refuses_a_key_the_password_does_not_derive`; [`tests/whitelist_seed.rs`](../tests/whitelist_seed.rs) | Same password protects encryption and policy edits: no second factor. A seed thief can bypass this signer. Same-user changes can deny service. |
| Compromised CLI lies about recipient/amount/token | Daemon independently decodes unsigned bytes and checks their meaning in [`EVM tx`](../src/chains/evm/tx.rs), [`EVM policy`](../src/chains/evm/policy.rs), [`Tron tx`](../src/chains/tron/tx.rs), [`Tron policy`](../src/chains/tron/policy.rs) | Decoder/policy unit cases and [`tests/policy_limits.rs`](../tests/policy_limits.rs), [`tests/tron_send.rs`](../tests/tron_send.rs) | Only supported canonical forms are accepted. Parser correctness and crypto libraries remain trusted; this is not formal verification. |
| Abuse ERC-20 approval/transferFrom or typed authorization | Decoded token transfer special cases, revoke-only approvals, transferFrom refusal; no EIP-712/raw-hash/type-4 signing path | EVM decoder/policy tests; [`tests/whitelist_daemon.rs`](../tests/whitelist_daemon.rs) | Generic allowed contract effects and existing on-chain allowances can still move value. The wallet does not revoke permissions held by other protocols automatically. |
| Drain funds through many individually permitted transfers | Optional signed native/token/calldata per-transaction limits checked on decoded bytes | `signed_limits_apply_to_the_daemons_payload_and_can_be_removed` | No cumulative budget or per-agent allocation enforcement. Repeated permitted actions can drain available funds and fees. [#26](https://github.com/2lab-ai/zwallet/issues/26) remains design-only by owner decision. |
| Use an allowed contract to route value somewhere else | Contract and selector allowlist plus decoded native/calldata limits | EVM policy unit cases; [current policy contract](../.prd/04-whitelist.md) | An allowlisted selector is not a semantic contract audit. Parameters, upgrades, proxies, arbitrary indirect effects, slippage and economic loss are not generally bounded. |
| Spend more in fees than advertised | EVM execution fee cap; Tron energy fee limit, resource-aware CLI balance/preflight | [`tests/policy_limits.rs`](../tests/policy_limits.rs); [`tests/tron_review.rs`](../tests/tron_review.rs) | Per transaction only. OP L1 data fees are outside the EVM execution cap. Tron bandwidth/activation/governance price changes are outside its signed energy cap. An estimate is not an all-in guarantee. |
| Abuse message signing for login/payment authorization | EIP-191 enabled only by an explicit signed flag | `w10_sign_message_needs_allow_message_signing` in [`tests/whitelist_daemon.rs`](../tests/whitelist_daemon.rs) | A permitted message may carry external economic meaning or be replayable by its relying protocol. zwallet does not understand every relying application's semantics. Leave disabled unless needed. |

Before a whitelist exists, EVM native self-transfers remain possible under default
fee caps; this can still spend fees. Token self-transfers require policy, and Tron
self-TRX is refused. When a sealed anchor exists, signing fails closed if neither the
current list nor the staged list verifies against it. An absent sealed anchor is a
different state: even if list files exist, it retains the default-cap EVM self-transfer
exception, including after restoration of a wallet from before its first policy edit.

## Transaction manipulation, replay and RPC compromise

| Attack / ambiguity | Implemented defense and evidence | Residual risk |
|---|---|---|
| Substitute signed bytes or a different signer after review | CLI recovers returned signer and compares exact intended operation before journal/broadcast. Tests: [`valid_signature_for_different_intent_never_broadcasts_or_journals`; `exact_payload_signed_by_another_key_never_broadcasts`](../tests/transfer_review.rs); [`signer_substitution_wrong_key_and_malformed_signature_never_broadcast`](../tests/tron_review.rs) | A signature issued by the daemon is itself usable; an attacker who obtains it need not use the cooperative CLI's broadcast path. |
| Raise fees/change balance or policy between review and confirmation | Immutable review fingerprint, rechecks and renewed review on material increase. Tests: `rising_fee_returns_new_review_and_zero_sign_tx`, `op_l1_rise_refuses_and_fall_signs_while_execution_fee_is_unchanged`, `same_generation_resigned_payee_change_signs_nothing`, Tron `fee_rise_never_signs_and_returns_a_new_review` | State can change after the last observation. RPC can lie consistently. Rechecking is not an atomic chain-state lock. |
| Retry after a timeout and pay twice | Journal-before-broadcast, per-account send lock, pending/duplicate guard, explicit retry/replacement paths in [`chains`](../src/chains/mod.rs) and [`replacement`](../src/wallet/replacement.rs) | Journal and lock are cooperative local state, editable by the agent's user. Broadcast error/timeout does not prove non-submission. Duplicate guard is advisory and explicitly overridable. |
| Change EVM replacement intent or nonce | Original transaction authentication and fixed nonce; typed review captures replacement kind. Tests in [`tests/replacement_review.rs`](../tests/replacement_review.rs) count signer calls | Replacing/cancelling competes with the original; neither success nor undo is guaranteed. Already mined transactions cannot be cancelled. |
| Replay a valid signature after lock or on another context | Supported EVM payloads bind chain ID and nonce; Tron binds its supported transaction fields/reference/expiry; decoder rejects unsupported forms | `lock` cannot retract issued signatures. Same-chain replay prevention ultimately depends on chain rules. EIP-191 application replay is outside transaction nonce protection. Forks, chain compromise and same-ID networks require separate trust analysis. |
| Feed stale/expired Tron data | CLI broadcast preflight checks freshness; strict raw decoding rejects malformed/unsupported fields | Daemon checks the permitted time window but does **not** reject every already-expired payload: a direct client can obtain such a signature. Chain acceptance and current node observation remain separate. |
| RPC impersonates a network or lies about balances/fees/receipt | Chain/genesis checks, expected-type validation and RPC deadlines; Tron and explorer responses additionally have 1 MiB body limits. Exact signing remains independent of node's declared intent | A malicious node can return the expected identity and fabricated state. The EVM wrapper has no explicit application-level response-body cap or whole-process memory bound. No light client, consensus proof or independent multi-node quorum is implemented. Outcome labels describe node observations. |
| Receipt wait hangs or late success overwrites timeout | Bounded in-flight request/sleep deadline. Tests: [`deadline_cancels_slow_receipt_rpc_without_accepting_a_late_success`; `deadline_bounds_the_sleep_before_another_poll`](../tests/review_receipt_deadline.rs) | Timeout means pending/uncertain, never proof of non-payment. Node withholding, reorgs and false receipts remain possible. |
| Explorer/address poisoning or token metadata lies | Source coverage and unverified labels; registry units for transfers; rendered external text escaping | [`tui_registry_token_send_uses_exact_registry_units_and_rejects_rpc_decimal_swap`](../tests/transfer_review.rs); [history tests](../tests/history.rs) cover the local boundary, not token issuer honesty, contract behavior or explorer completeness. |

An allowed `--dry-run` can request a **real signature**. It does not broadcast through
that CLI path, but it is not equivalent to an unsigned simulation or a secret-free
authorization exercise. Live tests and a funded owner wallet must not be substituted
for isolated mock/test-chain evidence.

## Local accounting, concurrency and denial of service

### Task reports and duplicate warnings are advisory

| Attack / false inference | Implemented defense and regression evidence | Residual risk |
|---|---|---|
| Change invoke arguments but hide the change behind the same destination/value | Duplicate matching hashes encoded calldata; signature/raw forms normalize to bytes. `invoke_duplicates_compare_calldata_and_keep_legacy_rows_conservative` in [`chains`](../src/chains/mod.rs) retains conservative legacy matching. | This is a local warning, not a semantic contract check or daemon replay boundary. Only the last 1 MiB of editable journal is searched. |
| Tamper duplicate-window setting or mistake its source for verified policy | Optional signed 1–86400-second policy field preserves old bytes when absent; authenticated edit/reset tests in [`tests/policy_limits.rs`](../tests/policy_limits.rs); successful and refused sends identify advisory source/window/tail. | CLI uses a local-file setting; TUI uses its password-checked review. A direct client or explicit override can bypass duplicate warnings. Actual signing still authenticates the whole policy. |
| Treat missing fees, unknown receipts or inconsistent replacement observations as available budget | Native task report retains six states, exact units, missing values and replacement identity; `forged_attribution_and_unknown_fees_never_yield_available_allocation` and `conflicting_mined_replacements_and_missing_receipt_proof_stay_uncertain` in [`task tests`](../src/wallet/tasks.rs) | Task files are mutable local attribution. Untagged/deleted records are not discovered; node observations are not consensus proof. Remaining allocation is advisory, not an authorization counter. |
| Leak account identity or personal notes through a report | Bounded task-ID/asset schema, no free-text descriptions/raw calldata/signed bytes, default address redaction and explicit reveal; `hostile_task_files_names_and_oversized_metadata_are_refused_without_echoing` | Redaction is not anonymity. Transaction hashes, task IDs and address-derived labels remain linkable; users must not put secrets or personal data into IDs. |

See [task report semantics](task-reports.md) for baseline/current provenance, unknown
fees, balance deltas and limitations. No report changes the retained #26/#27 design
decision or turns mutable local state into a trusted spending limit.

### Session and resource controls

| Attack | Implemented defense | Regression evidence | Residual risk |
|---|---|---|---|
| Rename/replace socket to hide an unlocked daemon or forge “locked” | Process-table-based all-session lock, SIGTERM+SIGCONT then bounded SIGKILL, fresh scan before success | `k2_lock_stops_a_daemon_whose_socket_was_swapped_for_a_fake`, `r4c1_lock_reads_no_home_and_stops_every_daemon`, `r2_lock_stops_a_daemon_behind_a_repointed_symlink` in [`tests/cli.rs`](../tests/cli.rs) | Requires trustworthy process enumeration/UID/signaling. Lock affects all this user's zwallet homes. It cannot erase already stolen keys or revoke signed transactions. |
| Hide processes through Linux `/proc` restrictions | Unlock/status/lock reject visibility they cannot safely use; no silent “locked” claim on a partial scan | `a_proc_mounted_with_hidepid_or_unreadable_hides_daemons`; `a_denied_read_of_this_users_process_fails_the_lookup` in [`protocol`](../src/core/protocol.rs) | Other OS/kernel/security-module behavior is still trusted. Root can defeat or kill the process. |
| Stall wallet/list reads so lock/expiry cannot run | Filesystem work outside session mutex, separate secret-free workers, four slots and 2-second deadline per worker class; final session checks before signing | [`lock_does_not_wait_for_wallet_or_socket_inspection`; `expiry_does_not_wait_for_wallet_or_socket_inspection`; `inspection_timeout_ends_session_and_late_success_cannot_sign`](../src/daemon/mod.rs) | Startup decryption has a different boundary. Wallet/socket inspection failure clears the session; list-read failure refuses that signature and can recover without re-unlock. Scheduling and arbitrary host availability are not guaranteed. |
| Late list read signs after lock, expiry or re-seal | Repeat session/ciphertext/expiry checks after read and under signing lock | `lock_is_answered_while_a_list_read_is_stalled_and_the_late_read_signs_nothing`; `a_read_that_ends_after_expiry_signs_nothing`; `a_read_during_which_wallet_json_is_resealed_signs_nothing` | An operation already signed before a later lock cannot be retroactively undone. Expiry uses host wall clock; backward changes or suspension alter elapsed lifetime. |
| Flood connections, large messages or blocked storage | 1 MiB message bound, 10-second connection I/O budget, bounded inspection/list workers | `inspection_workers_stay_bounded_when_queued_requests_time_out`; `outstanding_list_readers_are_bounded_and_signing_resumes_after_release`; [`c4_silent_connection_does_not_keep_the_seed_past_expiry`](../tests/cli.rs) | Accepted connections have no global task quota. CPU/RAM/fd exhaustion, malicious lock holding, SIGSTOP/kill, disk exhaustion and deletion can deny service. These tests do not prove whole-host availability. |

## Output, dependencies and distribution

| Attack | Implemented defense and evidence | Residual risk / absent defense |
|---|---|---|
| Library parse error echoes phrase; RPC/daemon text injects terminal controls | Fixed secret-parse errors, bounded relayed details, final terminal allowlist and JSON escaping in [`seed`](../src/core/seed.rs), [`protocol`](../src/core/protocol.rs), [`terminal`](../src/core/terminal.rs) | `invalid_phrase_error_does_not_echo_words`; `a_relayed_error_is_escaped_and_never_reads_as_this_clis_own`; `write_human_filters_what_it_writes`; CLI `the_write_point_escapes_text_no_source_escaped`. Escaping is not semantic secret redaction; do not put secrets into public labels or transaction inputs. |
| Panic or background output draws hostile content after TUI exit | Serialized terminal ownership, fixed panic line, bounded emergency restoration in [`term`](../src/tui/term.rs) | `the_emergency_path_takes_no_stdio_lock`; `a_second_panic_while_the_first_ends_the_process_parks_and_says_nothing`; [`failed_lock_then_quit_restores_before_exact_cli_error_output`](../src/cli/terminal_observation_tests.rs) checks the shared PTY stream through EOF. Terminal emulator/another process can still draw arbitrary output. |
| Dependency changes between builds | Checked-in [`Cargo.lock`](../Cargo.lock), CI/build `--locked`, pinned Rust 1.95.0 | [`validate.yml`](../.github/workflows/validate.yml) and build workflow source. These pin selected inputs; they do not prove that dependencies are benign, vulnerability-free or fully reproducible. No dependency advisory scan is claimed by this review. |
| Action tag retargeting or excessive workflow token access | Full commit SHAs for external actions; contents-read default; publish jobs alone receive contents-write; checkout does not persist credentials | [`ci.yml`](../.github/workflows/ci.yml), [`preview.yml`](../.github/workflows/preview.yml), [`release.yml`](../.github/workflows/release.yml). A compromised pinned action, runner, repository account or publish job can still alter artifacts. |
| Build/test failure still publishes a release | Publish depends on validation and all build jobs; exact revision passed into reusable validation; artifact version smoke tests | Workflow dependency graph and [`validate.yml`](../.github/workflows/validate.yml). This is source inspection plus actual CI for a submitted revision, not proof of branch protection/account settings or independent release reproduction. |
| Tampered download or malicious publisher | Release includes SHA256SUMS; macOS binaries receive ad-hoc hardened-runtime signing and flag verification | Release workflow source. A checksum fetched from the same compromised publisher is not independent authentication. Ad-hoc signing is not Apple notarization or a developer identity certificate. No separate provenance attestation/SBOM/reproducible-build guarantee is claimed. |
| Loader injection or debugger on macOS release | Release workflow sets hardened runtime and verifies flags; DYLD behavior check only runs where SIP is enabled | Preview/release workflow documents SIP-disabled runner limitation. Source builds may lack release hardening, and SIP/root/kernel compromise remain outside the protection. |

Cargo's [lockfile documentation](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)
explains dependency resolution pinning. GitHub's [secure-use reference](https://docs.github.com/en/actions/reference/security/secure-use)
explains action commit pinning and least-privilege workflow permissions. Neither
practice certifies the code being pinned.

## Claims this implementation does not support

- “Safe against all hacks,” “AI-proof,” formally verified or independently audited.
- Protection after password/seed theft, root/kernel compromise or executing a malicious wallet binary.
- Hardware isolation, a second factor, guaranteed memory erasure, or complete side-channel resistance.
- A trustworthy cumulative spending budget, per-agent isolation or rollback-proof local policy state.
- All-in fee ceilings, safe arbitrary contracts, guaranteed profit, stop-loss execution or merchant delivery.
- Trustless receipt/finality verification, guaranteed broadcast, non-payment proved by timeout, or cancel as undo.
- Human presence proven by a terminal, publisher authenticity proven by ad-hoc signing, or safety proven by a checksum alone.
- Post-quantum security: the implemented transaction/policy signatures use conventional elliptic-curve schemes. Architectural extensibility is not post-quantum protection.

The budget and freshness designs remain explicit future work in
[#26](https://github.com/2lab-ai/zwallet/issues/26) and
[#27](https://github.com/2lab-ai/zwallet/issues/27). Investment, service-payment and
merchant adapters depend on those unresolved boundaries and their own protocol
policies; their roadmap entries are not defenses present in this wallet.

## How to evaluate or report a security claim

For a proposed defense, identify the attacker capability, exact path and trust
assumption; reproduce it with an isolated `ZWALLET_HOME`, fixed test credentials and
mock/test-chain data; count signature/broadcast requests as well as output; then run
the repository gate and both platform CI jobs on the final source. Do not use real
funds to establish a negative security claim.

For a suspected vulnerability, preserve the version/commit, platform, public request
shape and minimal reproduction. Do not include a real phrase, password, private key,
credential-bearing RPC URL or usable signed transaction in a public report. This
document does not establish a private reporting channel or a disclosure policy that
the maintainers have not published.
