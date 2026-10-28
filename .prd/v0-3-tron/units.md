# v0.3 Tron — PRs, work items and order

Status: **planned**, 2026-10-28; the PR plan was revised after review round 1 the same day (the
dispatcher's decision from the three reviews). Four PRs, merged in this order: A → B1 → B2 → C.
Inside them are seven work items, TRON-1 … TRON-7, described below. Each PR closes the acceptance
IDs of [`../05-tron.md`](../05-tron.md) §Acceptance listed for it; every ID TR1–TR25 is closed by
exactly one PR, except TR22 (the TUI task) and TR23 (owner-gated, after C).

**No line cap.** The rule is that each PR can be reviewed as one argument. A PR may be read in
chunks, but no chunk boundary falls between what the decoder refuses and what the policy assumes,
and none falls inside C's chain from signing to the journal, the broadcast and the uncertain
states.

| PR | Work items | What the owner gets | Closes | After | Preview |
|---|---|---|---|---|---|
| **A, see it** | TRON-1, TRON-2 (no `tron` default fee cap) | the Tron address (verified after unlock), TRX and USDT balances | TR1–TR8, TR24 | — | yes, after merge |
| **B1, codec** | TRON-4 | the protobuf codec, txID, signature, the strict decoder with every refusal listed | TR12, TR13 | A | no |
| **B2, guard** | TRON-3, TRON-5, the `tron` default fee cap | Tron whitelist entries and the daemon's judgement. Its review evidence carries B1's strict-decode refusal list next to the judgement | TR9, TR10, TR11, TR14, TR15 | B1 | no |
| **C, send it** | TRON-6, TRON-7 | build, fee preflight, dry-run, broadcast, `--wait`, journal, lookup | TR16–TR21, TR25 | B2, and Q3 closed (below) | yes, after merge; then TR23 |

Why these cuts, from the reviews of round 1:
- A is usable on its own: the owner sees the address and both balances.
- B1 and B2 are different reviews: bytes against the protobuf definitions, then the policy against
  the threat model (fable). B2's review still reads B1's refusal list, because the policy is safe
  only under the decoder's invariants (grok).
- C is the send lifecycle, read as one (gpt-6-astra).
- Previews only after A and after C: each preview changes the binary's cdhash and costs the owner
  a `lock` and an `unlock` (grok).
- The `tron` default fee cap moves from A to B2 (fable): in A it would have no signer to bound,
  yet a first `add` or `reset` would write it, and older zwallet builds would stop editing that
  list for no gain.

**Counting rule** (information only, no cap; as [`../v0-2-tui/units.md`](../v0-2-tui/units.md)):
changed lines, insertions plus deletions, tests and `tests/fixtures/` included; `Cargo.lock`
excluded. The EVM modules these mirror measure, code + tests (`src/chains/evm/`, 2026-10-28):
`tx.rs` 478 + 400, `policy.rs` 474 + 863, `rpc.rs` 1,477 + 495, `registry.rs` 158 + 304, `mod.rs`
411 + 237.

**Every PR:**
- RED first: each test below is written and seen failing before the code it checks.
- At least one counter-example: input from outside (the node, `wallet.json`, `whitelist.json`, the
  daemon socket) built to get an address shown as payable or a transaction signed.
- "Nothing signed" is counted where the daemon side records it (`FakeDaemon.received`,
  `tests/common/mod.rs:587`), not only by the absence of a broadcast at the mock node.
- Tests that stop daemons hold `common::exclusive()` (`tests/common/mod.rs:95-99`).
- The Tron mock node (`tests/tron_offline.rs`, from A): an HTTP server on 127.0.0.1 keyed by path
  (`/wallet/getaccount`, …), the `MockRpc` pattern of `tests/evm_offline.rs:39-140`, recording
  every request body. Its clock is the head block it serves; tests set it apart from the local
  clock.
- No test searches for anything at gate time; every vector is fixed in the test.
- The gate of [`loop.md`](loop.md) §Build facts, then an outside review, then merge.

| Work item | Scope | Closes | PR | Est. lines (information) |
|---|---|---|---|---|
| TRON-1 | family, chain row, derivation, address codec, exact compare, address views and QR, chain identity, TRX balance | TR1, TR2, TR4, TR5, TR24 | A | 760 |
| TRON-2 | USDT registry and `balanceOf`, `--token` on `balance`, every-network row, owner-permission warning, `address --verify` adds missing chains | TR3, TR6, TR7, TR8 | A | 720 |
| TRON-3 | whitelist Tron entries: parse, store, show, `validate`'s two rules, `*` = EVM only, `@nick` | TR9, TR10, TR11 | B2 | 700 |
| TRON-4 | protobuf codec, txID, signature, recovery, golden fixtures | TR12, TR13 | B1 | 750 |
| TRON-5 | the daemon's judgement: `ListView` moved, Tron policy, `sign_tx`, the `tron` cap | TR14, TR15 | B2 | 780 |
| TRON-6 | transfer build, fee preflight (estimate and maximum), dry-run, signed-reply check, refused flags | TR16, TR17, TR18, TR19 | C | 850 |
| TRON-7 | broadcast, outcomes, `--wait`, journal, `tx --chain tron`, `expired-unseen` | TR20, TR21, TR25 | C | 900 |

## TRON-1 — family, derivation, address, TRX balance

- **Scope:**
  - `Family::Tron(tron::TronChain)`, the `tron` row in `CHAINS` (chainId 728126428, default RPC
    `https://tron-rpc.publicnode.com`, explorer Tronscan), a Tron arm at every
    `Family::Evm` match: `src/chains/mod.rs:34, 41, 174, 192, 198, 204, 1484`,
    `src/commands.rs:257, 1126, 1346, 1550, 1787, 1808`, `src/commands/whitelist.rs:978`.
    `transfer` and `tx` on `tron` return `CAPABILITY_UNSUPPORTED` until PR C; `sign-message`,
    `call`, `invoke` on `tron` return it for good.
  - `evm::config` (`src/chains/evm/mod.rs:40-44`) returns `Result`, refusing a non-EVM chain; its
    12 call sites gain `?`. A non-exhaustive match would otherwise be a panic in the daemon.
  - `src/chains/tron/{mod,address,rpc}.rs`: base58check parse and format (`0x41`, 34 characters,
    checksum), derivation `m/44'/195'/0'/0/i`, `FamilyKeys` (`sign_tx` refuses until TRON-5);
    `rpc`: the genesis check and `getaccount` (TRX balance, `activated`).
  - Exact (case-sensitive) comparison for Tron in `cache_mismatch` (`src/commands.rs:366-376`),
    `cache_repairs` and `moved_index` (`src/core/store.rs:705-800`); `address_shaped`
    (`src/commands.rs:255-267`) with a Tron arm.
  - `address --chain tron`, the `tron` row of `addresses` and its own QR with the Tron caption;
    `balance --chain tron` (TRX), the `every_balance` row (TRX, `tokens: []`).
  - No `tron` row in `DEFAULT_FEE_CAPS_WEI` (B2 adds it with the signer). Its test at
    `src/chains/evm/registry.rs:447` (`DEFAULT_FEE_CAPS_WEI.len() == CHAINS.len()`) becomes "one
    cap per EVM chain" here; B2 makes it every chain again.
  - A guard until B2: `whitelist add … --chains tron` and `whitelist fee-cap tron …` are
    `VALIDATION_ERROR` ("Tron payees and fee caps come with the Tron signer"). Without it, `tron`
    being a known chain would let a `0x` payee or a `tron` cap into a signed list early, and an
    older build's edits would refuse that list.
  - `Cargo.toml`: `bs58 = { version = "=0.5.1", features = ["check"] }`, `sha2 = "=0.10.9"` (both
    already in `Cargo.lock`).
  - README: the `tron` row of §Chains, `address --chain tron`.
- **Files:** `Cargo.toml`, `src/chains/mod.rs`, `src/chains/evm/{mod,policy,registry,rpc}.rs`,
  `src/chains/tron/{mod,address,rpc}.rs` (new), `src/commands.rs`, `src/commands/whitelist.rs`,
  `src/core/store.rs`, `src/qr.rs`, `tests/cli.rs`, `tests/tron_offline.rs` (new),
  `tests/live.rs` (`balance_without_chain_reads_every_network` writes `TWer2…` as the `tron`
  cache, not `0x…`), `README.md`.
- **RED tests:**
  - TR1 (unit): the two vectors; `m/44'/195'/0'/0/i` for `i` = 0, 1; index 2^31 refused.
  - (unit) the address codec: the USDT pair `TR7NHq…` ↔ `41a614…`; all 28 single-letter case
    flips of `TWer2…` inside the base58 alphabet, listed in the test (each fails the checksum,
    05 M11); a bad checksum; 33 and 35 characters; prefix `0x42`; a `0x` string: each refused.
  - (unit) `tron::address::same(a, b)` is byte equality: `TWer2…` against one of those case
    flips is false, without parsing either. The cache comparison calls it, never
    `eq_ignore_ascii_case`.
  - TR2 (cli): `create a` caches `addresses.tron` = the derived `T…`; locked `address --chain
    tron` → `verified: false`; unlocked (macOS signed build) → `verifiedBy: "session"`, no prompt;
    a QR only when verified.
  - TR4 (cli): the three tampered caches → `VALIDATION_ERROR`, escaped.
  - TR5 (mock): block 0 = Nile's id → `RPC_ERROR`, and the mock saw no `getaccount`.
  - TR24 (cli): `sign-message`, `call`, `invoke` with `--chain tron` → `CAPABILITY_UNSUPPORTED`.
- **Counter-examples:**
  - The daemon socket answers `address{chain: "tron"}` with an EVM `0x…`: `VALIDATION_ERROR`, not
    shown as a Tron address (`reply_address`, `src/commands.rs:285-301`).
  - `getaccount` with `balance` as a string, negative, or above 2^63: `RPC_ERROR`, nothing shown
    as a balance.
  - A cache that holds a case flip of the right `T…`: refused by the shape check (the checksum),
    and, if it ever passed, by the exact comparison. The two checks are tested apart above,
    on fixed inputs; no test searches for a variant that passes both.

## TRON-2 — USDT, owner permission, cache fill

- **Scope:** `src/chains/tron/registry.rs` (the USDT row with its source URL, `lookup`,
  `is_token`); `balanceOf` through `triggerconstantcontract`; `balance --chain tron --token
  usdt|<T…>` (`usdt` = the registry row; `<T…>` asks `symbol()` and `decimals()`); the USDT row in
  `every_balance`; the owner-permission `warning` on both; `address --verify` adds every chain this
  build derives that the cache lacks (`added: [{key, chain, now}]`) in `store::unlock_repairing`
  and `repair_under_lock` (`src/core/store.rs:808-880`), without the tamper warning; README
  `balance` notes.
- **Files:** `src/chains/tron/{registry,rpc}.rs`, `src/commands.rs`, `src/core/store.rs`,
  `tests/tron_offline.rs`, `tests/cli.rs`, `tests/live.rs`,
  `tests/fixtures/tron-account-taken-over.json` (new), `README.md`.
- **RED tests:**
  - TR3 (cli): a key with no `addresses.tron`: locked `address --chain tron` → `WALLET_LOCKED`
    naming unlock and `--verify`; `address --verify` → `added`, no `warning`; then cached.
  - TR6 (mock): the M7 `getaccount` answer of `TWer2…` (05 §Evidence), recorded as
    `tests/fixtures/tron-account-taken-over.json`: `warning` names `TDRqzjYye3bGaBtg1bAPw5F4duDchNSbtk`.
  - TR6 (live, `--ignored`): cache `TWer2…`; the test reads `getaccount` itself; `raw` equals its
    `balance`; `warning` is present exactly when its `owner_permission` is not this address alone
    at threshold 1. Neither the balance nor the key is pinned.
  - TR7 (live and mock): `--token usdt` → decimals 6, symbol from the registry; mock `balanceOf`
    error → the token row's error, TRX still shown.
  - TR8 (live): `balance` lists `tron` among the mainnets, Sepolia last; the `tron` row's
    `address` is the `T…`.
  - (live) `every_registry_token_answers_its_symbol_and_decimals` covers the Tron row.
- **Counter-examples:**
  - `balanceOf` answers 33 bytes, or `result.result: false` with a revert string of 600 `\u{1b}`:
    `RPC_ERROR`, the string escaped and within 512.
  - `owner_permission` names this address with `threshold: 2`, or two keys: the warning, and
    `--json` keeps the keys escaped.
  - `--token <T…>` whose `symbol()` is `USDT` but is not the registry contract: the symbol is
    shown escaped and marked as the contract's own, never as the registry's USDT.

## TRON-3 — whitelist Tron entries

- **Scope:** `add_change` (`src/commands/whitelist.rs:152-175`) parses `T…` (default `--chains
  tron`) and refuses the TR10 inputs; the stored form `0x` + body; `WHITELIST_RULES` and
  `DEFAULT_FEE_CAPS_WEI` move to `src/chains/mod.rs`, `is_registry_token` covering both families;
  `validate` (`src/core/whitelist.rs:294-324`, today only "every chain known" and "no contract
  entry at a registry token") gains two rules through a new `ChainRules::family_of`: an entry
  naming `tron` names no other chain, and it is an address entry, so a hand-built list breaking
  either is never signed; `Whitelist::payee(.., wildcard: bool)` (`src/core/whitelist.rs:255-259`);
  `T…` and "stored as 0x…" in the change summary, `family` and the `T…` form in `whitelist list`
  (human and `--json`); `@nick` for `tron`; A's guard removed; README §The whitelist (`*` = every
  EVM chain, Tron entries) and the release note line of 05 §The whitelist ("`*` in a whitelist now
  means every EVM chain; it never covers `tron` …").
- **Files:** `src/commands/whitelist.rs`, `src/core/whitelist.rs`, `src/chains/mod.rs`,
  `src/chains/evm/{registry,policy}.rs`, `tests/cli.rs`, `tests/whitelist_seed.rs`, `README.md`.
- **RED tests:**
  - TR9 (cli): `whitelist add bob <T…> --password-stdin`: the summary on stderr shows `T…` and the
    `0x` body; the file stores `0x…` and `["tron"]`; `list --json` → `address: "T…", family:
    "tron"`; `list --verify` → `verified: true`.
  - TR10 (cli): the six inputs (`T…` with `--chains base`, `T…` with `--chains '*'`, `0x…` with
    `--chains tron`, `--chains tron,base`, a bad checksum, `--contract` on `tron`) →
    `VALIDATION_ERROR` before the password; `whitelist.json` unchanged. (unit) `validate` refuses a
    list with `chains: ["tron", "base"]` and one with a contract entry on `tron`.
  - TR11 (unit and cli): a list file written by the current `main` with a `*` entry verifies;
    the golden-bytes test passes unchanged; `payee(body, "tron", false)` is `None`, `payee(body,
    "base", true)` the entry; `check_shapes` of a list with a `tron` entry passes.
  - TR11 (unit, EVM twin): with a verified list whose `bob` is `chains: ["tron"]`, the EVM
    `check_tx` refuses a native transfer and a registry `transfer` on `base` to `0x<bob's body>`
    with `WHITELIST_DENIED` and no `command`. Its `payee` path is `covers_chain`, unchanged from
    `main`, so this also pins what an older build does with such a list. TRON-5 repeats it
    through the daemon's `sign_tx`.
- **Counter-examples:**
  - A hand-edited `whitelist.json` adds a `tron` entry: the list does not verify, every
    signature is `WHITELIST_INVALID` (as W4), and `list` shows the entry's `T…` as unverified.
  - An entry with `chains: ["tron", "base"]` in a hand-written list: `validate` refuses it, so no
    edit signs it.

## TRON-4 — the transaction codec and signing primitive

- **Scope:** `src/chains/tron/tx.rs`: `encode_raw`, `decode_raw` (strict, re-encode equality,
  1 KiB limit), `txid` = sha256, `sign` (k256 over the txID, `r ‖ s ‖ v`, `v` ∈ {0, 1}, low
  `s`), `encode_signed`, `recover`; `DecodedTron::to_json` for `signerDecoded`.
  `tests/fixtures/tron-unsigned.json` (the two golden vectors) and `tron-signed.json` (the two
  mainnet transactions, `raw_data_hex` and `signature`).
- **Files:** `src/chains/tron/tx.rs` (new), `src/chains/tron/mod.rs`, `tests/fixtures/` (2 new).
- **RED tests:**
  - TR12 (unit): decode both golden vectors to the §The transaction fields; re-encode equals;
    `txid` equals the printed txIDs; sign one with the TR1 key: recovers to `TWer2…`, `v` ∈
    {0, 1}, low `s`; both mainnet signatures recover to their owners (`v = 0x1b` and `0x00`).
  - TR13 (unit): every input of TR13 → `VALIDATION_ERROR` "refusing to sign"; type 46 names the
    hand-over.
- **Counter-examples:**
  - The TRX golden vector with field 10 (a memo) inserted before field 11, lengths fixed up:
    refused. java-tron accepts a memo and charges `getMemoFee` for it.
  - The same vector with `amount` written as a 10-byte varint of the same value: refused (not
    minimal), though a protobuf parser reads the same number from it.

## TRON-5 — the daemon's judgement

- **Scope:** `ListView`, `NO_WHITELIST`, `MESSAGE_SIGNING_COMMAND`, `VERIFY_COMMAND` move to
  `src/chains/policy.rs` (EVM imports only; no behaviour change); `src/chains/tron/policy.rs`:
  `check_tx` (§The daemon's judgement steps 4–6); `FamilyKeys::sign_tx` for Tron: steps 1–7,
  returning `{rawData, signature, txHash, decoded}`; `("tron", "30000000")` in
  `DEFAULT_FEE_CAPS_WEI`, and its test back to "one cap per chain"; a Tron section in
  `04-whitelist.md` that points to 05 (one paragraph).
- **Review evidence:** B1's list of what `decode_raw` refuses (TR13, every level), placed next to
  `check_tx`, so the reviewer checks each assumption of the policy against a refusal of the
  decoder.
- **Files:** `src/chains/policy.rs` (new), `src/chains/evm/{mod,policy}.rs`,
  `src/chains/tron/{mod,policy}.rs`, `src/daemon/mod.rs` (imports, tests),
  `src/commands/whitelist.rs` (imports), `tests/whitelist_daemon.rs`, `.prd/04-whitelist.md`.
- **RED tests:**
  - TR14 (daemon, the `fixture` pattern of `src/daemon/mod.rs:682-760`): every TR14 input through
    `handle`, the codes and `command`s listed; a counter of signatures stays 0 for each refusal.
  - TR15 (unit): the five inputs; the `command` of each fee refusal.
  - TR11 (daemon): the EVM twin of a `chains: ["tron"]` payee through `handle`: refused, zero
    signatures.
  - (unit) the EVM policy corpus (`src/chains/evm/policy.rs` tests) passes unchanged after the
    move.
- **Counter-examples:**
  - USDT `transfer` whose `data` has a non-zero byte in the 12 padding bytes of the recipient
    word: refused (not canonical), never read as the 20-byte recipient.
  - A type-31 call to the USDT contract with selector `0xa9059cbb` and 100 bytes of `data`: refused.
  - A TransferContract whose `to_address` is 21 bytes starting `0xa0`, ending in `bob`'s 20-byte
    body: refused at decode (prefix), never matched to `bob`.

## TRON-6 — transfer: build, preflight, dry-run

- **Scope:** `tron::rpc`: `getblock`, `getaccountresource`, `getchainparameters`, the simulation;
  `prepare_transfer` (reference block, `timestamp` = its timestamp, `expiration` = `timestamp` +
  60,000 ms, `fee_limit` = ceil(`energy_used` × `getEnergyFee` × 1.2) and at least 1 TRX; the
  preflight with `estimatedCostSun`, `maxCostSun` and `pricesAtBlock`; `INSUFFICIENT_FUNDS`
  against `maxCostSun`; the owner-permission refusal); `inspect_signed` (`txHash` =
  sha256(`rawData`), `rawData` = the bytes sent, the signer recovered); `dry_run_check`'s Tron arm
  (`src/commands/whitelist.rs:971-1000`); `--nonce`, `--replace`, `--bump`, `--allow-pending`,
  `--max-fee-gwei` refused on `tron`; `--token` other than `usdt` refused on `tron`.
- **Files:** `src/chains/tron/{mod,rpc}.rs`, `src/commands.rs`, `src/commands/whitelist.rs`,
  `tests/tron_offline.rs`, `README.md` (`transfer --chain tron`, `fees`).
- **RED tests:**
  - TR16 (cli with `FakeDaemon`): the three bad replies → `VALIDATION_ERROR`; the mock saw no
    `broadcasthex`; no journal file.
  - TR17 (mock): dry-run to `@bob` → `fees`, `feeLimit`, `whitelist: {allowed: true}`, `txHash`,
    `recoveredFrom`, no raw bytes in the output; to an unlisted `T…` → `whitelist: {allowed:
    false, …}`, zero `SignTx`.
  - TR18 (mock): the five cases → `INSUFFICIENT_FUNDS` ×4 and `RPC_ERROR`, zero `SignTx`; the
    case with a balance above `estimatedCostSun` and below `maxCostSun` names `maxCostSun` and the
    `shortfall` against it.
  - TR19 (cli): the five flags, and `--token <T…>` → `VALIDATION_ERROR`, the mock saw no request.
  - (unit) the built `raw_data` has `timestamp` = the reference block's and `expiration` =
    `timestamp` + 60,000; `fees.maxCostSun` = amount + `fee_limit` + bytes × price (+ activation
    for a new payee), whatever staked or free resources the account has. The golden vectors do not
    test this: the node built them with its own clock.
- **Counter-examples:**
  - The mock answers `getEnergyFee` 10^15: the computed `fee_limit` is above the cap, and the
    dry-run reports the `FEE_CAP_EXCEEDED` refusal before asking the daemon.
  - The mock's head block is 10 hours old (its `timestamp`): the CLI refuses before signing
    (`RPC_ERROR` "the node's head block is 10 h old"): the node is behind, and a transaction built
    on that block would be expired on arrival.
  - The simulation answers `energy_used` as a string of 600 digits: `RPC_ERROR`, not a fee.

## TRON-7 — broadcast, outcomes, `--wait`, journal, lookup

- **Scope:** `broadcast` (`/wallet/broadcasthex`) and the outcome mapping of §Send safety; the
  journal's Tron arm (`expiresAt`, no `nonce`, txID and `T…` shapes in `is_address` and
  `entry_hash`, `src/chains/mod.rs:708-724`); file names with the `41…` hex; `wait_for_receipt`
  (`gettransactioninfobyid`, `gettransactionbyid` `ret`; `feeSun` from the receipt);
  `lookup_tx` (`tx --chain tron`: mined, pending, not found, `expired-unseen` by 05 §Send safety:
  the node's head block as the clock, a node more than 60 s behind the local clock never gives
  `expired-unseen`, an id outside the journal is `not found`; the returned `raw_data` hashed and
  compared); the duplicate and `BROADCAST_UNCERTAIN` messages with the second-payment sentence; a
  wording test that scans `src/` and `.prd/05-tron.md`, README for text calling a resend safe (as
  04's round-8 test does for restore hints); explorer URLs (close Q1 with one live Tronscan page
  read); README §The send journal and `tx`; one line in `02-architecture.md` §Send safety pointing
  to 05.
- **Files:** `src/chains/mod.rs`, `src/chains/tron/{mod,rpc}.rs`, `src/commands.rs`,
  `tests/tron_offline.rs`, `tests/live.rs`, `README.md`, `.prd/02-architecture.md`.
- **RED tests:**
  - TR20 (mock): TRX and USDT to `@bob --wait`: the `broadcasthex` body decodes to the signed
    transaction; txHash = sha256(raw_data); one journal line with `expiresAt`; the USDT receipt
    `SUCCESS` with 32 zero bytes of `contractResult` → `success: true`; `REVERT` → `success:
    false`.
  - TR20 (mock): a USDT receipt with energy above the simulation and within `fee_limit` →
    `success: true`, `feeSun` above `estimatedCostSun`, within `maxCostSun`; `OUT_OF_ENERGY` →
    `success: false` with `feeSun` and the burned-fee message.
  - TR21 (mock): the six cases, each with its code and fields.
  - TR25 (mock): the five lookups (a)–(e), with the node's head block and the local clock set
    apart in each; the wording test.
  - (live) `tx --chain tron 21b776c7…a690` → mined, `success: true`, the token transfer's `to`.
- **Counter-examples:**
  - The mock's broadcast answers `result: true` with `txid` of another transaction and then
    reports that one mined: the output's `txHash` stays the computed one, with `rpcTxHash` and a
    `broadcastWarning`; it never reports the other one as this send.
  - A journal line planted with a `txHash` of 64 hex digits plus `\u{1b}[2J`: shown as
    `<invalid>`.
  - The tx was mined before its expiration, but the mock answers `{}` for it and serves a head
    block 30 s past `expiresAt`, while the local clock is 5 min past it: `not found` (the node's
    head, not the local clock, decides; 60 s margin), never `expired-unseen`, and no message
    suggests sending again.

## TR22 and TR23

- **TR22** is closed by the TUI task when its coverage matrix lists every row of 05 §TUI.
- **TR23** runs after C is merged and a preview is released: the owner funds the key's Tron
  address and decides to send; the dispatcher runs the two sends and reads Tronscan; Q2 closes
  with it.

## Before C starts: Q3

One `POST /wallet/broadcasthex` to each of `tron-rpc.publicnode.com` and `api.trongrid.io`,
carrying an **unsigned** transaction (an `encode_raw` output with no signature; no node can
include it, java-tron refuses it, `TransactionCapsule.java:654-656` "miss sig or contract"). A
`SIGERROR` reply shows a keyless POST reaches validation, which closes Q3. A 401 or 403 from the
default means C changes the default endpoint first. Not done while writing the spec: its brief
allowed read-only calls only.
