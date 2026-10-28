# v0.2 TUI — current flow (what the TUI would call today)

Status: **in-progress**, 2026-10-27, read from `origin/main` c7d3f13 (PR #12), revised after
review rounds 1 and 2 the same day. For every TUI behaviour in [`../03-tui.md`](../03-tui.md)
this page names the code that does it today, with `file:line`, what it returns, and whether the TUI can
call it as it is. Every line reference is to that commit, except §Functions added on 2026-10-28
(24f4b5b). The units TUI-1 … TUI-12 in [`units.md`](units.md) close the gaps named here.

## Labels

- **exists**: a `pub` function the TUI can call as it is. It takes plain values (a home path, a
  `&Chain`, a password as `&str`, a `Duration`), not a clap argument struct; it reads nothing from
  `/dev/tty`; and it writes nothing to stdout or stderr.
- **exists, needs a library entry point**: the logic is there, but the TUI cannot call it as it
  is. It sits inside a private CLI command function, or it reads the password itself, or it
  prints. It must be split out into a `pub` function. The CLI command then calls that same
  function, so there is still one implementation (03-tui.md §Non-goals).
- **missing**: no code does this today.

**Library entry point** = a `pub` function in the `zwallet` library crate (`src/lib.rs`). The
splits below go into one new module, `src/wallet.rs`: the operations the CLI and the TUI share.
Each CLI handler then keeps only its argument parsing, its password prompt and its printing.

## Facts that apply to every row

1. **The command handlers are private.** `src/commands.rs` exports only `run` (commands.rs:48)
   and `close_inherited_fds` (:44). Every handler is a private `async fn` that takes a clap
   struct and returns `Out` (cli.rs:436: a JSON value, human text, and a QR code). The whitelist
   handlers are `pub(super)` or private (commands/whitelist.rs:51, :86, :971).
2. **The password is read inside the handlers.** `read_password` (core/password.rs:116) prompts on
   `/dev/tty` (`prompt_tty`, :152) or reads stdin. It is called by `unlock` (commands.rs:1923),
   every whitelist change (commands/whitelist.rs:263-268), `whitelist list --verify` (:1346),
   `address --verify` (commands.rs:955), `list --verify` (through `open_with_password`, :641) and
   `backup` (:1988). The TUI owns the terminal in raw mode, so it reads the password into its own
   buffer and must hand it to the library as a value.
3. **Four places write to the terminal from inside the logic.** In the TUI's full-screen mode
   such a write draws over the frame:
   - `SendRecord::write` prints the `broadcasting <hash> ...` line through `eprint_line`
     (chains/mod.rs:1198, :1215). `Evm::broadcast` calls it right before the send request
     (chains/evm/mod.rs:332), on every live send.
   - `create` prints the backup note to stderr (commands.rs:752-755).
   - A whitelist change with `--password-stdin` prints its summary to stderr
     (commands/whitelist.rs:264).
   - `prompt_tty` writes the password prompt to `/dev/tty` (core/password.rs:152).

   A library entry point for the TUI returns such text instead of printing it.
4. **Runtime and crates.** The CLI runs one current-thread tokio runtime (cli.rs:528); tokio has
   the features `rt`, `macros`, `time`, `net`, `io-util`, `signal`, `sync` (Cargo.toml:29).
   `ratatui` and `crossterm` are not dependencies (Cargo.toml:16-29; `cargo tree` on 2026-10-27
   lists neither). `reqwest` 0.13.5 is already in the tree through alloy (`cargo tree -i
   reqwest`), and the code reaches it as `alloy::transports::http::reqwest` (chains/mod.rs:14).
5. **There is no `zwallet tui` command and no `--warm` flag.** The `Command` enum has `Chains`,
   `Create`, `List`, `Change`, `Address`, `Addresses`, `Balance`, `Call`, `Invoke`, `Transfer`,
   `SignMessage`, `Tx`, `Status`, `Backup`, `Unlock`, `Lock`, `Whitelist` and the hidden
   `Daemon` (cli.rs:36-88).

## T1 Keys

| Behaviour | Code today | Returns | Status | Split (new entry point) |
|---|---|---|---|---|
| Read the key list | `store::read_wallet_file` (core/store.rs:538) | `WalletFile { active, next_index, keys: [KeyEntry { name, index, addresses }] }` (store.rs:131, :146), unverified metadata; key names and `active` already pass `validate_key_name` at parse (store.rs:568-576) | exists | none |
| Each row's address | `list` without `--verify` (commands.rs:825-887): the daemon's derivation through `Daemon::derive` and `check_cache` (:840-844), else the cache (`primary_address`, :185) | `{ active, keys: [{ name, index, address, active }], verified: false }` (:878); a cache the daemon contradicts is `VALIDATION_ERROR` | exists, needs a library entry point | `wallet::keys(home)`: the rows and the warning, no text |
| Mark the globally active key | `WalletFile.active` (store.rs:134) | the name | exists | none |
| Lock state, once, in the status bar | see **Status bar** below | | | |

## T2 Warm-up and T2b make active

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Checks before the password: the duration, the socket path, Linux `ptrace_scope`, `/proc` hidepid, and the daemon binary pinned so one swapped in while the human types is never started | `unlock` (commands.rs:1913-1922): `parse_duration`, `protocol::socket_path` (core/protocol.rs:161), `read_wallet_file`, `check_ptrace_scope` (:452), `check_daemons_findable` (:491), `DaemonExe::resolve` (:676) | `DaemonExe` | exists, needs a library entry point (the steps are `pub`, their order lives in the handler) | `wallet::prepare_unlock(home, ttl, allow_ptrace_scope_0) -> UnlockPlan`, called when the password field opens |
| Start the session with the typed password | `protocol::start_daemon(home, exe, password, ttl)` (core/protocol.rs:1215) | readiness `{ locked, expiresAt, active, index, address, pid, whitelist }` (daemon/mod.rs:364-372) | exists | `wallet::unlock(plan, password) -> Session`: adds the index warning (`session_index_warning`, commands.rs:1955) and `inert_whitelist` (:208) as `unlock` does (:1926-1944) |
| Which address the warm-up verifies | readiness carries the **globally active** key's address (daemon/mod.rs:367-369); `unlock` reports it `verified: true, verifiedBy: "password"` (commands.rs:1935-1936) | | exists | not enough for a key that is not the active one: the TUI's own password check (next section) derives the open key's address too |
| Session verification of an address | `Daemon::address` (commands.rs:540) then `Daemon::trust` (:555): `Trust::Session` only when every answer came from this build's key daemon of the home (`protocol::check_peer`, core/protocol.rs:1078); `Trust::Unverified(Some(warning))` when another process answered (`PeerCheck::warning`, :1042) | `(address, Trust)`; `Trust::mark` writes `verified`, `verifiedBy: "session"`, `warning` (commands.rs:386-394) | exists, needs a library entry point (`Daemon` and `Trust` are private) | `wallet::address(home, key, chain) -> (String, Trust)`, with `Trust` made `pub` |
| Session verification on Linux | `check_peer_linux` (core/protocol.rs:1170) returns `Unknown` for the genuine daemon, whose `/proc/<pid>/exe` is hidden (:737-742, :745-751) | never `Trust::Session` on Linux | exists | none; the status bar says so (03-tui.md §Address trust and session verification) |
| T2b `s` make active | `change` (commands.rs:930-947): checks the target's address through the daemon, then `store::set_active` (core/store.rs:1388) | `{ active, address, verified, verifiedBy?, warning? }` and a QR code (`with_qr`, :599) | exists, needs a library entry point | `wallet::set_active(home, name) -> (String, Trust)`; no QR (03-tui.md §Display integrity) |

## Password check (03-tui.md §Password check)

What the TUI runs with a password typed in this session (warm-up, whitelist change, `V`), in one
`tokio::task::spawn_blocking` task (the `rt` feature, Cargo.toml:29). The CLI runs the same steps
in `list --verify` and `whitelist list --verify`, each inside a handler that reads the password
itself (Fact 2). `address --verify` is not part of it: it repairs a modified cache
(`store::unlock_repairing`, commands.rs:956-957) instead of refusing one, and reads no list file;
it keeps its own path.

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Open wallet.json with the password and check every cached address | `store::unlock(home, password, derive)` (core/store.rs:643), as `open_with_password` (commands.rs:639-644) and `whitelist list --verify` (commands/whitelist.rs:1347) call it | `(Unsealed { seed, policy }, WalletFile)`; a modified cache is `VALIDATION_ERROR` | exists | none |
| The open key's and the active key's addresses from the seed | `derive_checked(seed, key, chain)` (commands.rs:647-651, private) | the address, checked against the cache | exists, needs a library entry point | `wallet::open_with_password(home, password, keys) -> Opened { addresses, anchor: Option<WhitelistAnchor>, salted: bool }`: `store::unlock`, `derive_checked` for each named key, the sealed anchor (`WhitelistAnchor`, core/store.rs:419) and whether a salt is sealed with it; never `SealedWhitelist` with its salt (core/store.rs:461-467): the salt does not leave the blocking task; the seed dropped before it returns. `list --verify` calls it with every key (commands.rs:827-829, :839) |
| The sealed anchor, and a zwallet 0.1.0 anchor's message | `Unsealed::sealed_whitelist` (core/store.rs:255); `list --verify`'s salt check and message (commands/whitelist.rs:1353-1360, `whitelist::without_salt`, core/whitelist.rs:1246) | `Option<SealedWhitelist { anchor, salt }>` | exists; the message needs a library entry point | the anchor: `wallet::open_with_password`; the message: `wallet::check_list` |
| Read both list files once and pick the list the seal names, verified | `ListFiles::read` (core/whitelist.rs:987), `pick_base_from` (:1097), which runs `verify` (:766: format, generation, ed25519 signature by the sealed public key, digest); `verified_view` (commands/whitelist.rs:1373-1403) does this for `list --verify`, but reads the files itself and keeps no fingerprint | `Base { list, source, discarded }` (core/whitelist.rs:1078) | exists, needs a library entry point | `wallet::check_list(home, anchor, salted) -> Result<CheckedList, String>`; `whitelist list --verify` calls `open_with_password` then `check_list` |
| The fingerprint of the bytes that verified | `ListFiles::fingerprint` (core/whitelist.rs:995): keccak-256 of each file's bytes (:970-975) | `Fingerprint` | exists | none |
| The checked list, as the TUI keeps it | none | | **missing** | `CheckedList { list, entries, policy, gen, source, fingerprint }`, returned by `wallet::check_list` and kept by the TUI; `policy` is the list's own (`Whitelist.policy`, core/whitelist.rs:230: fee caps, message-signing flag), not the wallet's `Unsealed.policy` (core/store.rs:241-244); no salt |
| Still the bytes that verified: when a pane that shows the checked list opens or refreshes, when the picker opens, before the review, at confirm | `whitelist::check_unchanged(home, &fingerprint)` (core/whitelist.rs:1022) | `VALIDATION_ERROR` naming the file that changed | exists | none |
| The session's whitelist view, as information only | `protocol::status` (core/protocol.rs:434-436) is a plain `send(Status)`: no peer check. `live_daemons` keeps the reply of any pid `daemon_pids` lists for the home (commands.rs:1884-1903), found by argv (core/protocol.rs:1749), which a process of this user can choose (tests/cli.rs:3905-3912 starts such a process) | `{ gen, verified, entries }` (daemon/mod.rs:123-138) | exists, needs a library entry point (`wallet::status`) | never a source of an address or of the word "verified" (03-tui.md §Display integrity rule 7) |

## Status bar

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Locked or unlocked, the expiry, the daemon pid | `status` (commands.rs:1804-1872) and `live_daemons` (:1884-1910): the process table (`protocol::daemon_pids`, core/protocol.rs:1749) decides; `d.sock`'s status reply (`protocol::status`, :434) is used only when it comes from a listed pid and says unlocked | `{ locked, daemonPids?, otherDaemonPids, daemonPid?, expiresAt? (escaped, at most 64, commands.rs:1827), whitelist? (inert, :1829), active, address, verified, verifiedBy?, warning? }` | exists, needs a library entry point | `wallet::status(home) -> StatusView` |
| The active key's address and what verifies it | inside `status` (commands.rs:1832-1856) | `(address, Trust)` | exists, needs a library entry point | part of `wallet::status` |
| RPC host, redacted | `rpc_url` (commands.rs:74; flag > `ZWALLET_RPC_<CHAIN>` > default, chains/mod.rs:179) then `redact_url` (chains/mod.rs:490) | `https://host` or `https://host/<redacted>` | exists (`redact_url`, `Chain::resolve_rpc`); `rpc_url` is private but is two lines | `wallet::net(chain) -> Net` |

## T10 Lock detection and the `L` key

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Notice `zwallet lock` from another shell | `wallet::status` above, read again on each refresh; the process table is the truth, not `d.sock` (commands.rs:1878-1883) | `locked: true` once no daemon of the home is left | exists, needs a library entry point | same as **Status bar** |
| `L` locks every daemon | `protocol::stop_all_daemons` (core/protocol.rs:1783), what `zwallet lock` calls (commands.rs:1971-1978) | the pids it stopped | exists | none |

## T3 Home: balance and the last 10 transactions

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Balance summary on the current chain | `Evm.wallet_balances(net, owner, limit)` (chains/evm/mod.rs:259) through `read_network` (commands.rs:1328); the owner address from `Daemon` as in `every_balance` (:1168-1190) | `(Balance, Vec<TokenBalance>)`: the native coin and every registry token of the chain, zero balances included; a token read that fails is that token's `Err` (chains/mod.rs:545-551) | exists (the trait method is `pub`), the owner lookup needs a library entry point | `wallet::balances(home, key, chain) -> ChainBalances`, with the owner's `Trust` |
| One network's error on its line | `error_text` and `brief` (commands.rs:1350-1362): the first line, cut to 72 characters (`BRIEF_ERROR_CHARS`, :1143) | text | exists, needs a library entry point | `wallet::brief` made `pub` |
| Journal rows (this machine's sends) | `SendJournal::entries` (chains/mod.rs:965) is private. The only `pub` readers are `entries_at(nonces, except)` (:998) and `recent_duplicates(send, now)` (:1064) | `Vec<Value>`, raw lines | exists, needs a library entry point | `SendJournal::recent(limit) -> Vec<JournalRow>` |
| Each journal field in the shape zwallet writes, else `<invalid>` | `EntryText::of` (chains/mod.rs:756, private): hash, nonce, time re-rendered, kind, amount (`is_amount_value`, :726), recipient | `EntryText` | exists, needs a library entry point | `JournalRow` (`pub`), built by the same checks |
| Explorer rows | none | | **missing** | `chains::evm::explorer::fetch(base, chain, address)` with the shape check in 03-tui.md §History |
| Merge, de-duplicate, order, first 10 | none | | **missing** | `history::merge(journal, explorer) -> Vec<HistoryRow>` |
| Coverage note per chain | none; the reachable explorers are a measured fact (03-tui.md §History) | | **missing** | `chains::EXPLORERS`: per chain, the explorer base URL or none, next to `CHAINS` (chains/mod.rs:77) |
| A journal row's status, looked up on chain | `Evm.lookup_tx(net, hash)` (chains/evm/mod.rs:404) → `rpc::lookup_tx` (chains/evm/rpc.rs:1290) | `{ chain, txHash, found, status: "mined" \| "pending" \| "not found", from, to, nonce, value, valueRaw, symbol, explorerUrl, blockNumber?, success?, gasUsed?, hint? }` (rpc.rs:1320-1371) | exists | none for the status |

## T4 Tx detail

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Hash, from, to, nonce, block, status | `rpc::lookup_tx` (chains/evm/rpc.rs:1290): addresses re-checksummed by zwallet (:1301-1305), numbers parsed from hex (:1375) | see T3 | exists | none |
| Native value | `value` is the native `value` of the transaction only (rpc.rs:1309-1311, :1328-1331) | | exists | none |
| Token movements that involve the wallet | the receipt is read as plain JSON (`fetch_receipt`, rpc.rs:1408, private), but `lookup_tx` keeps only its `gasUsed` and `status` (:1339-1346); no code in `src/chains/evm/` reads a receipt's `logs` or decodes an ERC-20 `Transfer` event. A transfer routed through another contract has the token amount and the real recipient only in those logs | | **missing** | `rpc::tx_detail` decodes the receipt's `Transfer` logs of registry tokens on this chain (`registry::lookup`, chains/evm/registry.rs:109) whose from or to is the wallet: several movements, a reverted transaction (none), a pending one (unknown); a movement of amount 0 is counted, not shown |
| Fee `gasUsed x effectiveGasPrice` | only `gasUsed` is read from the receipt (rpc.rs:1342-1346); no `effectiveGasPrice` | | **missing** | `rpc::tx_detail` reads `effectiveGasPrice` and, on base and optimism, the receipt's `l1Fee` |
| Confirmations | no head block is read in `lookup_tx` | | **missing** | `rpc::tx_detail` reads `eth_blockNumber` |
| Explorer URL | `Chain::explorer_tx_url` (chains/mod.rs:196) | the explorer link | exists | none |
| Nickname of from/to if listed | `Whitelist::by_address` (core/whitelist.rs:248) | `Option<&Entry>` | exists | used only on the checked list (§Password check), never on `read_list`'s unverified file |
| "local record" for this machine's sends | the journal line (`SendRecord::new`, chains/mod.rs:1175: `{ ts, chain, from, nonce, txHash, to, value, kind, amountRaw?, token? }`) | | exists, needs a library entry point | the `JournalRow` of T3 |
| The daemon-decoded action (03-tui.md T4) | the daemon's `decoded` view of what it signed is returned with the signature and shown once as `signerDecoded` (commands.rs:1702-1705); the journal does not keep it (chains/mod.rs:1175-1192) | | **missing** after the send | 03-tui.md T4 is revised to show the journal's fields only (§Changes) |

## T5 Assets

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Native coin plus registry tokens with a non-zero balance | `Evm.wallet_balances` (chains/evm/mod.rs:259) | registry symbol (`USDC`, `USDT`, `DAI`, `WETH`, chains/evm/mod.rs:297), decimals, raw; the TUI hides zero rows | exists | `wallet::balances` (T3) |
| Which tokens a chain has | `registry::TOKENS` (chains/evm/registry.rs:52-99): 19 rows; bsc has none | | exists | none |

## T6 Transfer and the status of a send

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Payee picker: address entries valid on this chain | the checked list (§Password check); `Entry::covers_chain` (core/whitelist.rs:141) and the entry's `kind` (:97-104) | entries the password checked | exists, needs a library entry point (`wallet::check_list`) | the picker reads only the checked list; `whitelist::read_list` (core/whitelist.rs:860) alone is never a source of a payee |
| `@nickname` → address | `recipient` (commands/whitelist.rs:86-102), unverified on purpose | `(address, Some(nickname))` | exists, needs a library entry point | not needed: the picker holds the entry |
| Amount → base units | `parse_units` (core/amount.rs:8) | `U256` | exists | none |
| Review: fee estimate | `rpc::prepare` (chains/evm/rpc.rs:625) through `prepare_transfer` (:1074): `gasLimit`, `maxFeePerGas`, `maxGasCost`, `l1Fee`, `nativeBalance`, `sufficientFunds`, `shortfall` (:761-810) | `PreparedTx { unsigned, fields }` (chains/mod.rs:608) | exists | none |
| Review: the pending guard | `prepare` refuses when an earlier transaction is pending (`pending_guard_error`, rpc.rs:672-676, :848), unless `allow_pending`; a dry-run lists `pendingNonces` instead (:670) | error, or fields | exists | the review runs `prepare` as a dry-run |
| Review: the duplicate guard | `SendJournal::recent_duplicates` (chains/mod.rs:1064) with `send_summary` (commands.rs:1596, private) | matching journal lines | exists, `send_summary` needs a library entry point | part of `wallet::review_transfer` |
| Review and confirm: the whitelist verdict and the fee cap | the CLI's `dry_run_check` (commands/whitelist.rs:971, `pub(super)`) runs `policy::check_tx` (chains/evm/policy.rs:65) on whitelist.json as it is on disk (:980) | `NotChecked` / `Allowed` / `Denied({ allowed: false, reason, code, ... })` | `policy::check_tx` and `ListView::Usable` (policy.rs:41-55) exist and are `pub` | the TUI calls `policy::check_tx` on `ListView::Usable(&checked.list)`, the bytes the fingerprint holds, never `dry_run_check`'s fresh read (03-tui.md §Status of a send) |
| Review without asking the daemon to sign | a CLI `--dry-run` asks the daemon to sign once the list allows it (commands.rs:1680-1716) | | exists, needs a library entry point | `wallet::review_transfer(home, key, net, payee, amount, token) -> Review`: prepare (dry-run), duplicates, whitelist check; no signature, no lock. `Review` holds the exact intent (03-tui.md §Status of a send) |
| Confirm signs only what the review held | `tx::decode_unsigned` (chains/evm/tx.rs:378) decodes an unsigned transaction: `chain_id` (:329), `nonce` (:333), `to` (:346), `value` (:350), `input` (:354), `action` (:358: `NativeTransfer { to, value }` or `TransferCall { token, to, amount }`, :164-172). Nothing compares two prepared transactions today | | **missing** (the decoder exists) | `Review::matches(&prepared)` inside `wallet::send_transfer`: the transfer intent exactly equal, the most the fee can cost (`max_gas_cost` + `l1_fee`, U256, chains/evm/rpc.rs:736-744) at most the reviewed one, else nothing is signed. `prepare` keeps those two only as formatted strings in its fields (:797-804), so it returns the integers too |
| Send: lock, prepare, sign, check the signer, journal, broadcast | `transfer_with` (commands.rs:1538-1589) and `sign_and_send` (:1631-1748): `SendLock::acquire` (chains/mod.rs:651), `prepare_transfer`, `protocol::send_unlocked(SignTx)` (core/protocol.rs:427), `inspect_signed` and the signer check (commands.rs:1690-1696), `broadcast` (chains/evm/mod.rs:316) | `Out` with `txHash`, `nonce`, `explorerUrl`, `journal` | exists, needs a library entry point (takes `&SendArgs`, returns `Out`, and the broadcast prints to stderr: Fact 3) | `wallet::send_transfer(...) -> Sent`, and `SendRecord::write` returns its line instead of printing it |
| Hash at once, then follow the status | `wait_for_receipt` (chains/evm/mod.rs:391) → `rpc::wait_receipt` (rpc.rs:1443), polling every 3 s (`RECEIPT_POLL`, :28) | `{ status: success \| reverted \| mined, success, blockNumber, gasUsed }` or `None` at the timeout | exists | none |
| `dropped/replaced`: its nonce mined by a different hash | `lookup_tx` reports only `mined`, `pending` or `not found` (rpc.rs:1315-1319). `mined_at` (rpc.rs:965) finds the transaction at a mined nonce, but it is private and serves only `check_nonce` (:1009). The mined nonce is read only inside `prepare` (rpc.rs:654-664) | | **missing** | `rpc::tx_state(net, hash, from, nonce)`: `dropped/replaced` when the account's latest nonce is past `nonce` and the hash is not mined |
| Polling bound reached: `unknown — run zwallet tx` | `record_receipt` (chains/mod.rs:1391) turns a receipt that did not come in time into `status: "pending"` with a `hint` (:1396-1399), not `unknown` | | **missing** on the TUI side | the TUI's send-status state (`src/tui/app.rs`, TUI-5) maps its own 90 s bound to `unknown`; `record_receipt` is not used |

## T7 Whitelist

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| List the entries (gen, verified state, policy) without the password | `unverified_view` (commands/whitelist.rs:1416) through `list` (:1343) | `{ gen, entries, policy, verified: false, source?, reason? }`; the human text bounds every file value (`list_text`, :1459) | exists, needs a library entry point | `wallet::whitelist_view(home)` |
| The session's view of the list | the daemon's `summary` (daemon/mod.rs:123-138) in its status reply (:608-615) and readiness (:371), relayed through `inert_whitelist` (commands.rs:208); no peer check on that reply (§Password check, last row) | `{ gen, verified: true, entries: [nicknames] }` or `{ gen, verified: false, entries: [], reason }` | exists, needs a library entry point | part of `wallet::status`; information only: `gen` and a count, never an entry string, never "verified" |
| The verified state: check with the password (`V`, a change, the warm-up) | `list --verify` (commands/whitelist.rs:1345-1362): `store::unlock` (core/store.rs:643), `sealed_whitelist`, `verified_view` (:1373) | `{ ..., verified: true, source, sealedGen }` or the reason | exists, needs a library entry point (reads the password) | `wallet::open_with_password` then `wallet::check_list` (§Password check); `V` starts no session and writes nothing |
| A change, step 0: check the arguments, run `unlock`'s checks when `--unlock` is given, read both list files once, build the summary of what the password authorizes | `edit` (commands/whitelist.rs:240-262): `add_change` (:152), `ListFiles::read` and `fingerprint` (core/whitelist.rs:987, :995), `summary` (commands/whitelist.rs:405) | the summary text, the fingerprint | exists, needs a library entry point (the password prompt follows in the same function, :263-268) | `wallet::whitelist_plan(home, change, unlock) -> EditPlan { summary, fingerprint, ... }`, `Change` made `pub` |
| A change, steps 1-5 with the typed password: check it against wallet.json, pick the base, try the change, check the files are the bytes shown, stop every daemon, edit under `w.lock`, then the new session if `--unlock` was given | `edit` (commands/whitelist.rs:269-318): `store::unlock` (:272), `base_for_edit` (core/whitelist.rs:1155), `edit_salt` (:1256), `try_change` (commands/whitelist.rs:183), `check_unchanged` (core/whitelist.rs:1022), `protocol::stop_all_daemons` (:290), `edit_as_shown` / `reset_as_shown` (core/whitelist.rs:1317, :1345), `land_then_unlock` (commands/whitelist.rs:326) | `{ change, gen, previousGen, base, entries, verified: true, stoppedDaemons, entry? \| feeCap? \| dropped?, discarded?, warning?, locked, hint? }` plus `{ duration, expiresAt, daemonPid, whitelist }` with `--unlock` (`edit_output`, :1017) | exists, needs a library entry point | `wallet::whitelist_apply(plan, password) -> EditOutcome` |
| The whitelist key | derived again on every edit from the password and the salt sealed in wallet.json: `WhitelistKey::derive` (core/whitelist.rs:727) → `store::derive_whitelist_secret` (core/store.rs:391); the first edit and every reset draw a new salt (core/whitelist.rs:1399-1402). There is no key file (04-whitelist.md:104) | | exists | none |
| add (address entry, or contract entry with selectors) | `add_change` (commands/whitelist.rs:152): the nickname rule, the EIP-55 checksum (`evm::parse_address`, chains/evm/mod.rs:81), chains, selectors, no registry token as a contract | `Change::Add(Entry)` | exists, needs a library entry point | `Change::add(nickname, address, chains, selectors)` |
| remove | `Change::Remove` (commands/whitelist.rs:58-61, :132) | | exists, needs a library entry point | `Change::remove(nickname)` |
| fee cap | `Change::FeeCap` (commands/whitelist.rs:62-70): amount in the chain's coin, more than zero | | exists, needs a library entry point | `Change::fee_cap(chain, amount)` |
| reset | `Change::Reset` (commands/whitelist.rs:79); `reset_as_shown` (core/whitelist.rs:1345) | | exists, needs a library entry point | `Change::reset()` |
| allow-message-signing | `Change::MessageSigning` (commands/whitelist.rs:71-78) | | exists | not a TUI behaviour (03-tui.md T7 lists add, remove, fee cap and reset) |

## T8 Chain switch

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| The chains to pick from | `CHAINS` (chains/mod.rs:77), `chains::find` (:150) | 8 chains | exists | none |
| The default chain | `PRIMARY_CHAIN` = `ethereum` (chains/mod.rs:145). Nothing stores a "last chain used" (03-tui.md §Changes) | | exists | none |
| Every pane reloads for the new chain | the `Net` for the chain (Status bar row), then T3's calls again | | exists, needs a library entry point | `wallet::net` |

## T9 and T11 (terminal and password)

| Behaviour | Code today | Returns | Status | Split |
|---|---|---|---|---|
| Raw mode, alternate screen, restore on exit and panic | none (no `ratatui`/`crossterm`: Fact 4) | | **missing** | `src/tui/term.rs` |
| One write point for cells, at run time | the CLI's is `write_human` (core/terminal.rs:44) with `terminal_ok` (:24); nothing writes terminal cells today | | **missing** | `FilteredBackend` in `src/tui/term.rs`: every cell not exactly one `terminal_ok` character becomes `?`, counted; a two-column glyph's second column becomes a space; the backend's own cursor and style sequences pass (03-tui.md §Display integrity rule 1) |
| Panic hook's line | `eprint_line` (core/terminal.rs:57) writes through the filter | | exists | the hook prints one fixed line with it, never the payload |
| Password buffer: fixed capacity, wiped | `MAX_SECRET` = 1024 bytes (core/password.rs:13); `into_password` (:97) and `Zeroizing` buffers | | exists for the limit; the TUI's own buffer is **missing** | `src/tui/password.rs` |
| No core dump while the password is held | `disable_core_dumps` runs first in every command (cli.rs:473, core/password.rs:23) | | exists | none (`zwallet tui` is a command like the others) |

## Functions added on 2026-10-28

For the rows of [`../03-tui.md`](../03-tui.md) §Coverage matrix that the 2026-10-20 plan did not
have. Read from `origin/main` 24f4b5b (TUI-1a and TUI-1b merged); `wallet.rs` now exists with
`keys`, `status` and `net` (wallet.rs:40, :103, :155).

| Behaviour | Code today | Status | Split | Unit |
|---|---|---|---|---|
| Every network's balances | `every_balance` and `read_network` (commands.rs:1173-1350) | exists, needs a library entry point | `wallet::every_balance(home, key)` | TUI-3 |
| Every family's address | `addresses` (commands.rs:1054) | exists, needs a library entry point | `wallet::addresses(home, key)` | TUI-4 |
| Repair a modified cache from the seed | `address --verify`: `store::unlock_repairing` (commands.rs:965-1052, :973) | exists, needs a library entry point (reads the password, :971) | `wallet::repair(home, password) -> Repaired` | TUI-4 |
| Every key's address from the seed | `list --verify` (commands.rs:846-908) through `open_with_password` (:660) | exists, needs a library entry point | TUI-2's `wallet::open_with_password` with every key | TUI-4 |
| `--allow-ptrace-scope-0` for the TUI's sessions | `check_ptrace_scope` (core/protocol.rs:452) takes the flag; `Command::Tui` has no such flag (cli.rs:83-93) | **missing** in the TUI | a flag on `Command::Tui`, passed to `wallet::prepare_unlock` | TUI-4 |
| Overrides and a fee ceiling on the review | `SendArgs` (cli.rs:242-280); `max_fee_wei` (commands.rs:1532) | exists, needs a library entry point | parameters of `wallet::review_transfer` | TUI-5 |
| Message-signing flag change | `Change::MessageSigning` (commands/whitelist.rs:112) | exists, needs a library entry point | `Change::message_signing` | TUI-6 |
| Lookup by a typed hash | `tx` (commands.rs:1783) → `lookup_tx` (chains/evm/rpc.rs:1290) | exists | none (the detail is `rpc::tx_detail`, TUI-8) | TUI-8 |
| Replacement at nonce N | `replacement` (commands.rs:1521-1531) reads `--nonce`, `--replace`, `--bump`; `transfer_with` (:1554) | exists, needs a library entry point | `Replacement` in `wallet::review_transfer` | TUI-9 |
| Create the master seed or a key | `create` and `add_subkey` (commands.rs:683-845); the backup reminder printed (:773-776) | exists, needs a library entry point (reads the password, prints) | `wallet::create(...) -> Created` with the reminder as text | TUI-10 |
| The recovery phrase | `backup` (commands.rs:1970-1995) | exists; a TUI entry point only if trade-off B picks B2 | `wallet::recovery_phrase(home, password)` | TUI-10 |
| `call` | `call` (commands.rs:1388-1436) | exists, needs a library entry point | `wallet::call` | TUI-11 |
| `invoke` | `invoke` (commands.rs:1437-1508) and `sign_and_send` (:1647) | exists, needs a library entry point | `wallet::review_invoke`, `wallet::send_invoke` | TUI-11 |
| `sign-message` | `sign_message` (commands.rs:1792-1818) | exists, needs a library entry point | `wallet::sign_message` | TUI-11 |
| A token by contract | `balance --token` (commands.rs:1106-1172), `erc20_meta` (chains/evm/rpc.rs:209) | exists, needs a library entry point | `wallet::token_balance` | TUI-11 |
| Tron | none on main; 05-tron.md is being written (branch `docs/tron-spec`) | **missing** | per 05-tron.md | TUI-12 |
