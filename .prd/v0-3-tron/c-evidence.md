# Tron C implementation evidence

Status: in-progress, 2026-10-29. This is observed source/mock evidence, not funded live acceptance.

## Q3: unsigned keyless validation

At 2026-10-29T07:04:46.440185Z, the dispatcher used the production User-Agent
`zwallet/0.1.0`, `POST /wallet/getblock` with `{"detail":false}`, then
`POST /wallet/broadcasthex` carrying a fresh protobuf Transaction envelope with zero
signature fields. Both endpoints reached signature validation:

| Endpoint | Reference block | Block timestamp (ms) | HTTP | Result |
|---|---|---|---|---|
| https://tron-rpc.publicnode.com | 87528058 | 1793257482000 | 200 | `SIGERROR`: miss sig or contract |
| https://api.trongrid.io | 87528059 | 1793257485000 | 200 | `SIGERROR`: miss sig or contract |

An earlier Python-default-User-Agent read received Cloudflare HTTP 403 at PublicNode;
the production-client probe above supersedes that reachability inference. PublicNode
remains the default. This proves keyless validation access, not successful signed broadcast.
No funded transfer was attempted; TR23 is owner-run.

## Local verification

An initial reference-block construction test was observed failing with the unimplemented
constructor. After implementation the reference timestamp and exact 60,000 ms expiry test,
and dynamic fee branch/resource tests pass.

`cargo +1.95.0 test --locked --test tron_send`: 15 tests passed on 2026-10-29, using only
127.0.0.1 nodes and isolated `/tmp/zwt-*` homes. These cover malformed signer responses,
exact envelope/signature recovery, dynamic activation maximums, resource loss, insufficient
funds, node lag, simulation failure, actual receipt outcomes, broadcast classifications,
wait deadlines, dry-run refusals with zero SignTx requests, duplicate guards and journal fields.
The 124 chain-library tests also pass, including existing EVM journal regressions.
Strict all-target clippy passes with warnings denied. The ignored historical USDT read test
passed against PublicNode: fixed txID `21b776c7…a690`, block 87494658, mined, successful,
correct owner and a decoded token recipient distinct from the token contract. No signing
or broadcasting occurs in that test.
An additional historical-transaction regression passes: raw bytes outside the signing subset
retain the node's execution result without inventing decoded transfer details. Existing Tron
read regressions also pass (13 tests), including credential redaction and VM failure refusal.
The initial sandbox run was unable to bind local sockets; the same focused target passed
outside that sandbox. This is not a claim that the entire repository gate has run.

## Remaining evidence

Q1 rendered Tronscan page verification was attempted but no browser surface is available in
the worker (`iab` unavailable; browser inventory empty). Explorer links retain their existing
specified route. Q2 and funded TRX/USDT TR23 remain explicitly owner-run. The dispatcher owns
final integration gates, independent review and the separate TUI TR22 work.

## Review correction: unknown execution results

The reviewer identified that missing or unfamiliar execution results were incorrectly labelled
failed. The new boundary test was observed RED, then GREEN after making `success` tri-state:
SUCCESS is true, recognized failure enums are false, and missing/null/default/unknown/future
results are null with an execution-unknown and second-payment warning. Both native ret and
contract receipt paths are covered. Known REVERT and OUT_OF_ENERGY behavior remains false.

The independent review also caught a pre-observation deadline incorrectly labeled `pending`. The slow-genesis regression failed with `pending` before the fix. An unfinished first observation now yields `status: unknown, found: null`; a completed `not found` observation survives the later deadline. No receipt absence is inferred from a timeout.

## Parent final integration gate

On 2026-10-29, the final integrated B2/C source passed Rust 1.95.0 fmt --check,
locked all-target Clippy with warnings denied, and cargo test --locked:
**591 passed, 0 failed, 14 explicit live-test skips**. All 19 tests in tron_send passed.
The added real daemon + CLI acceptance uses isolated temporary wallets and a loopback
mock node to sign both TRX and USDT, independently recover both signers, broadcast
only to that node, observe a receipt, and verify the pre-broadcast journal/expiration.
No funded or signed public-network transaction was sent.

The first integration gate caught the old A/B2 CLI assertion that transfer and tx were
unsupported. The C-specific capability test was restored while retaining the executable-copy
race repair: sign-message/call/invoke still reject, and C's separate 19 tests exercise transfer
and lookup. The entire gate was rerun successfully; no failing test was skipped.
Local receipt: /tmp/zwallet-c-final-gate.log. Exact-head Linux/macOS CI remains pending.
The independent review corrections above and current B2 daemon rejection corpus are retained.
