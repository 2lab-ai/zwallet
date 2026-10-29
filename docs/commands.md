# Command reference

[Back to README](../README.md) · [Feature status](features.md) · [Security and custody](security.md)

This reference describes the current CLI. For a first wallet, start with the
[quick start](../README.md#quick-start).

## Commands

Every CLI command except `tui` takes `--json`. On failure with `--json` the CLI prints
`{ "error": "<msg>", "errorCode": "<CODE>" }` on stdout (plus `txHash` or `refusedTxHash`,
`nonce`, `from`, `journal` after a send, see **Error codes**) and exits 1; without `--json` the
message goes to stderr as `error: <msg> [<CODE>]`, followed by one `txHash: …` /
`refusedTxHash: …` / `nonce: …` / `from: …` / `journal: …` line for each of those fields, exit 1.
Commands that act on a key take `--key <name>` (default: the active key).

| Command | Output (`--json`) | While locked |
|---|---|---|
| `chains` | `[{ key, name, family, symbol, chainId, rpc }]` | works |
| `create <name> [--password-stdin] [--unlock <dur> [--allow-ptrace-scope-0]] [--words 12\|15\|18\|21\|24] [--no-qr]` | `{ name, index, address, active, master, home, chains, verified }` (+ `locked, expiresAt, daemonPid, whitelist` with `--unlock`) | see below |
| `list [--verify [--password-stdin]]` | `{ active, keys: [{ name, index, address, active }], verified, verifiedBy?, warning? }` | works |
| `change <name> [--no-qr]` | `{ active, address, verified, verifiedBy?, warning? }` | works |
| `address [--chain <key>] [--verify [--password-stdin]] [--no-qr]` | `{ chain, key, address, verified, verifiedBy?, warning? }` (EIP-55; on `tron` the base58check `T…`; `--verify` adds `index`, and `added?`/`repaired?` when it fills or repairs the cache) | works |
| `addresses [--no-qr]` | `{ key, addresses: [{ chainId: <key>, address }], verified, verifiedBy?, warning? }` (a row with no cached address while locked, such as `tron` for a key made before this version, has `error` and `errorCode` instead of `address`) | works |
| `status [--no-qr]` | `{ wallet, locked, home, daemonPids?, otherDaemonPids, daemonPid?, expiresAt?, whitelist?, active?, address?, verified?, verifiedBy?, warning? }` | works |
| `balance [--no-qr]` | `{ key, address, verified, verifiedBy?, warning?, chains: [{ chain, chainId, native: { symbol, decimals, balance, raw }, tokens: [{ symbol, address, decimals, balance, raw }] }] }`; a network that failed has `{ chain, chainId, error, errorCode }` instead, a token whose read failed `{ symbol, address, decimals, error, errorCode }` (see below) | works |
| `balance --chain <key> [--token <erc20>] [--rpc <url>] [--no-qr]` | `{ chain, key, address, symbol, decimals, balance, raw, verified, verifiedBy?, warning?, token? }` (on `tron` also `activated`, and `--token usdt\|<T…>` adds `symbolSource`) | works |
| `call --chain <key> --to <contract> --sig "<fn>" [ARGS...] [--rpc <url>]` | `{ chain, from, to, sig, args, result, raw }` | works |
| `invoke --chain <key> --to <contract\|@nick> (--sig "<fn>" [ARGS...] \| --data 0x...) [--value <dec>] [--dry-run] [--wait [--wait-timeout <dur>]] [--nonce <n> [--bump <factor>] [--replace <txHash>] \| [--allow-pending] [--allow-duplicate]] [--max-fee-gwei <gwei>] [--rpc <url>]` | transfer's send fields + `{ sig, args, data, value, valueRaw, toNickname? }` | `WALLET_LOCKED` |
| `transfer --chain <key> --to <0x\|@nick> --amount <dec> [--token <erc20>] [--force] [--dry-run] [--wait [--wait-timeout <dur>]] [--nonce <n> [--bump <factor>] [--replace <txHash>] \| [--allow-pending] [--allow-duplicate]] [--max-fee-gwei <gwei>] [--rpc <url>]` | see below | `WALLET_LOCKED` |
| `sign-message [--chain <key>] --message <s>` | `{ chain, key, address, message, signature, digest }` (EIP-191; refused unless the whitelist allows it; `CAPABILITY_UNSUPPORTED` on `tron`) | `WALLET_LOCKED` |
| `tx --chain <key> <txHash> [--rpc <url>]` | `{ chain, txHash, found, status, from, to, nonce, value, valueRaw, symbol, explorerUrl, blockNumber?, gasUsed?, success?, hint? }` | works (no wallet needed) |
| `unlock <dur> [--password-stdin] [--allow-ptrace-scope-0] [--no-qr]` | `{ locked: false, duration, expiresAt, active, index, address, daemonPid, verified: true, verifiedBy: "password", whitelist, warning? }` | - |
| `lock` | `{ locked: true, wasUnlocked, stopped: [<pid>] }` | - |
| `backup` | prints the recovery phrase; interactive terminal only | asks for the password |
| `whitelist list [--verify [--password-stdin]]` | `{ gen, entries: [{ nickname, kind, address, chains, selectors? }], policy: { maxFeeWei, allowMessageSigning, transactionLimits? }, verified, source?, sealedGen?, reason? }` | works |
| `whitelist add <nick> <0x> [--chains <c1,c2>\|'*'] [--contract --selectors <0x…,0x…>] [--unlock <dur>]` | a change (see **The whitelist**) + `{ entry, warning? }` | asks for the password |
| `whitelist remove <nick> [--unlock <dur>]` | a change + `{ entry }` | asks for the password |
| `whitelist fee-cap <chain> <amount> [--unlock <dur>]` | a change + `{ feeCap: { chain, maxFeeWei, maxFee } }` | asks for the password |
| `whitelist native-cap <chain> <coin amount\|none> [--unlock <dur>]` | a change + `{ transactionLimit: { chain, target, value, perTransaction: true }, policy }` | asks for the password |
| `whitelist token-cap <chain> <token address> <integer base units\|none> [--unlock <dur>]` | a change + `{ transactionLimit: { chain, target, value, perTransaction: true }, policy }` | asks for the password |
| `whitelist calldata-cap <chain> <byte count\|none> [--unlock <dur>]` | a change + `{ transactionLimit: { chain, target, value, perTransaction: true }, policy }` | asks for the password |
| `whitelist allow-message-signing on\|off [--unlock <dur>]` | a change + `{ allowMessageSigning }` | asks for the password |
| `whitelist reset [--unlock <dur>]` | a change + `{ dropped }` | asks for the password |
| `tui [--chain <key>] [--warm <dur>] [--allow-ptrace-scope-0]` | none: a terminal UI for a human (`--json` is refused) | works |

## Key management and terminal UI

- **Whitelist changes** also take `--password-stdin`, and with `--unlock <dur>` also
  `--allow-ptrace-scope-0` (see **The whitelist**).
- **Durations** are positional: an integer and one unit of `s m h d w y` (`1y` = 365d). `0`,
  negatives, fractions and unknown units are `VALIDATION_ERROR`.
- **`tui`** (needs stdin on a terminal, the one it sets to raw mode, and stdout on one): the
  key list with the globally active key marked `*`, and a two-line status bar: chain, open
  key, its short address and how far it is verified; then `locked` or `signing (agent too) until
  HH:MMZ` (UTC; `YYYY-MM-DD HH:MMZ` when a day or more away), and the RPC host. The lock state is
  read again every 5 s, so a `zwallet lock` from another shell shows. Keys: `j`/`k` or the
  arrows move. Enter opens a key without changing the global active key; while locked it asks
  for the password, starts a signing session (`--warm 1h` by default), then checks the open and
  active key's addresses and the signed list in the TUI. The password is masked, never echoed;
  Esc cancels it. `--allow-ptrace-scope-0` accepts Linux scope 0 at session start, as for `unlock`.
  Addresses checked with the password stay pinned until the TUI exits; a later conflicting
  daemon or cache answer stops the pane, preserving the pinned address in the bar. `s` asks, then `y` makes the selected key the globally active one, the one the
  agent uses by default (as `zwallet change`: a key whose cached address the daemon's answer
  contradicts is refused, the error in the pane), `r` reads everything again, `L` locks every
  zwallet daemon of this user (as `zwallet lock`), `q` or Ctrl-C quits (after a running `L` is
  done; activation and password checks finish too; a failed operation is printed once the terminal is given back). The note under the list
  never asks to verify the key the bar shows verified by session; it then flags the other keys,
  as `list` does. It needs 80x24 and says so on one line when the terminal is smaller. Text from
  wallet.json, the daemon socket and the environment is escaped and bounded as in the CLI, every
  cell goes through one filter that lets only printable ASCII and `₮` through, and no QR code is
  drawn. Tabs `1`–`7` and `[`/`]` navigate; `?` or `:` opens the 55-action menu. Typing in
  a field or menu keeps `q`, `?` and digits as input; Esc closes it. Actions belonging to later
  work are listed with their disabled reason.

  `2 wallet` shows non-zero native/registry-token balances; `a` reads every network, keeping
  partial failures, and Esc returns to one network. `c` chooses a chain; `e` in the picker
  enters an RPC URL for this process only. It is accepted only after a chain-id check; the
  field hides credentials, path/query and key-like host labels. No endpoint is saved to disk.
  `4 receive` shows the full address, its trust and every network address. `Q` points to
  `zwallet address` for a terminal QR code. `V` verifies all keys from the seed on `1 keys`,
  and the open plus active key elsewhere, without changing a signing session. `A` on `1 keys`
  repairs cached addresses and shows old/new values. `u` asks for a duration and then the wallet
  password before starting that signing session. `7 session` shows the home, expiry, daemon
  pids, and the daemon's reported whitelist generation/count as information only. The startup
  ptrace flag cannot be changed by any TUI key. Contract and message tools
  remain ![WIP](https://img.shields.io/badge/status-WIP-yellow).
  See [feature status](features.md#planned-features).
  On `1 keys`, `n` creates another key; an empty wallet home opens the first-wallet form.
  Enter a key name, choose 12/15/18/21/24 seed words on first creation (Left/Right), optionally
  enter an unlock duration, then type the name again to confirm. Preparation precedes the
  password field. A new wallet asks for the new password twice; another key asks once.
  The created key becomes active for the agent. A submitted creation finishes before quitting
  or locking; a failed unlock may follow a successful file write, so the key list is refreshed.
  `B` points to `zwallet backup` in a shell; the TUI never displays a recovery phrase.

  `3 send` reviews and sends EVM native/registry-token transfers using the pinned open key.
  `5 whitelist` lists the policy; `V` checks it locally with the password before selection or
  a message toggle. `a` adds a payee, `C` adds an EVM contract with selectors, `d` removes the
  selected entry, `f` sets the fee cap, `n`/`t`/`b` set native/token/calldata transaction limits,
  `m` toggles message signing and `R` resets the policy. Every edit shows its exact scrollable
  summary before asking for the password, stops all zwallet daemons before writing, then
  verifies the saved list locally. A blank duration leaves the wallet locked; an explicit
  duration requests a new session and its outcome is shown. `L` during saving locks after the
  edit completes; `q` waits for the edit and that lock. Zero-address adds are refused before
  the password; legacy zero entries remain viewable/removable and unavailable as payees.

  On the wallet tab, `Tab` switches between balance scrolling and history selection, Enter opens
  a transaction, and `/` accepts a `0x` + 64-hex-digit EVM hash. The ten-row history combines this
  home's selected-key journal with fixed Blockscout sources on Ethereum, Optimism and Sepolia;
  other chains show journal coverage only. Source labels and unavailable-explorer notes remain
  visible. Detail shows node claims, fee (including supported L1 fee), confirmations and only
  registry-token receipt movements; `j`/`k` scroll, `r` refreshes and Esc returns. A missing receipt
  with a consumed journal nonce is `dropped/replaced`, not proof that another payment is safe.
  Nicknames appear only from a password-checked, unchanged whitelist.
  On Tron, `/` also accepts the displayed 64-digit txID without `0x`. History lists this
  machine's sends only; it does not index incoming transfers. Detail verifies returned raw bytes
  against the hash and shows the full registry token contract, burned sun, expiry and unknown
  execution results. It offers no speed-up, cancel or resend control.

  `3 send` on Tron offers TRX and registry USDT to an exact Tron-only checked nickname. Review
  shows the held full addresses, integer token units, bandwidth/energy/activation costs,
  maximum TRX cost, conservative TRX left, reference-price block and expiration. Enter requires
  the exact nickname before signing. A fee maximum increase returns a new review. `d` explicitly
  allows a repeat and keeps the second-payment warning visible; EVM nonce/gwei controls are absent.
  The status reports the locally computed txID and journal expiration. `w` extends its receipt
  deadline. `L` and `q` wait for a submitted send to finish before locking or leaving.
  The four `tron ...` action-menu rows use these same handlers when Tron is selected. On an
  EVM chain they say `select Tron with c`, preserving the current endpoint and key selection.

- **Tron** ([Tron design](../.prd/05-tron.md)): this version shows the Tron address of each
  key, reads TRX and USDT, and sends both under the Tron whitelist policy. `create` caches `addresses.tron`, the
  key's address at `m/44'/195'/0'/0/<index>` (key 0's is the one TronWeb derives by default from the
  same phrase), as base58check `T…` (34 characters, payload `0x41` and the 20-byte body, a
  4-byte double-SHA-256 checksum). Tron addresses are compared byte for byte, never ignoring case:
  a cached value that is no such address (a case flip fails the checksum) is `VALIDATION_ERROR`,
  shown escaped. A key made before this version has no cached Tron address: while locked
  `address --chain tron` is `WALLET_LOCKED` (unlock once, or run `zwallet address --chain tron
  --verify`, which adds it and says so in `added`), and its `addresses` row shows that error. After
  `unlock`, `address --chain tron` is verified by the session as for EVM, with no prompt.
  `sign-message`, `call` and `invoke` on `tron` are `CAPABILITY_UNSUPPORTED`.
  `whitelist add bob <T-address> --chains tron` adds a payee; `*` covers EVM only, never Tron.
  Tron contract entries and selectors are unsupported.
  A first edit/reset adds the default 30-TRX fee cap; `whitelist fee-cap tron 30`
  sets it explicitly. Historical JSON fields named `maxFeeWei` and `maxNativeWei`
  store sun on Tron (1 TRX = 1,000,000 sun). Token caps use smallest token units,
  and calldata caps bound bytes. These caps remain per transaction, not cumulative.
  The daemon accepts only strict native transfers and the registry USDT's exact transfer call,
  from its own derived address to a listed Tron payee, under the signed limits. It refuses
  approvals, arbitrary contracts, permissions, memo, malformed protobuf and self-TRX.
  `transfer --chain tron --to @bob --amount 1 [--token usdt] [--dry-run] [--wait]`
  sends only TRX or registry USDT. A contract address is accepted for token balance reads,
  but transfer's token flag accepts only `usdt`. Nonce/replacement/bump/pending/gwei flags
  are refused before RPC. `--force` cannot bypass the daemon's payee policy.
  Preflight refuses unactivated or changed-owner senders, failed simulations and insufficient
  TRX for `fees.maxCostSun`, even when `fees.estimatedCostSun` is affordable. Prices and
  resource availability come from the node; `fees.pricesAtBlock` identifies the head read
  during this preflight. Maximum cost assumes resources are spent by another transaction
  first, but does not bound a governance price change. `feeLimit` bounds contract energy;
  bandwidth and new-account activation are protocol fees outside that signature bound.
  Dry-runs return independently verified `signerDecoded`, `txHash` and `recoveredFrom`, never
  raw broadcastable bytes. The CLI compares the daemon's raw payload to the requested bytes.
  The send journal uses the 21-byte `41…` hex address in its filename, records `expiresAt`
  and no nonce, and is fsynced before broadcast. The ten-minute duplicate guard remains active.
  `tx --chain tron <txid> [--wait [--wait-timeout 30s]]` accepts 64 hex digits with or without
  `0x`, verifies returned raw bytes against the requested hash, and shows burned `feeSun`.
  `expired-unseen` needs a known journal expiration, no mined/pending record, and a fresh
  node head more than 60 seconds past expiration. Its `asOf` identifies the block, time and
  host; a lagging node instead reports `nodeBehindSecs` and suggests another RPC.
  This node's absence report is not proof of non-payment. Sending this payment again is a
  second payment if this one was included. A failed execution can still burn fees. Missing or
  unrecognized execution results stay `success: null` with an execution-unknown warning; a
  mined receipt alone does not prove whether the payment succeeded.

- **`create <name>`**: on first run it creates the master seed (BIP39, 24 words by default) and key
  `<name>` at index 0, under a new password of at least 12 characters. On a terminal it then
  reminds the human (on stderr) to run `zwallet backup` before funding the wallet. The home's
  parent directory must already exist. Afterwards it derives the next index as `<name>` and makes
  it active. At a terminal or with `--password-stdin` it asks for the password for that one
  operation and derives the key itself (`verified: true`). Without either (an agent) it derives
  through the unlocked daemon without a password (`verified: false`), and while locked it refuses.
  It starts no daemon unless `--unlock` is given. Concurrent creates get distinct indices.
## Address verification

- **Address trust** (`verified`, `verifiedBy`): the addresses in `wallet.json` are a plain cache
  that anything running as the same user can edit, and a listener of that user can answer on
  `d.sock`. An address is shown at one of three levels:
  - `verified: true, verifiedBy: "password"`: derived from the seed after the human's password,
    by this process or by the daemon child it just started.
    - `address --verify` and `list --verify` (password at the terminal prompt, or
      `--password-stdin`): decrypt `wallet.json` here, derive, and compare with the cache. A
      wallet the password does not open is `WRONG_PASSWORD`. With the password the seed is
      authoritative and the cache only metadata, so `address --verify` **repairs** every cached
      address (of every key) that differs from the seed: it rewrites them in `wallet.json` under
      the writer lock (the encrypted master untouched), and says so in `repaired: [{key, chain,
      was, now}]` (`was` escaped) and a `warning` that something modified `wallet.json`. `list
      --verify`, `unlock`, `create` of a further key with the password, `backup`, the whitelist
      changes and `whitelist list --verify` refuse such a cache with `VALIDATION_ERROR` and say to
      run `zwallet address --verify`.
      Both show each key's derivation `index`, and add a `warning` when an index is one zwallet
      never assigns (at or past `nextIndex`, or with an unused index below it): the key list in
      `wallet.json` was edited. Such an address is still this seed's, but a wallet restored from
      the phrase lists accounts in index order and may not show it.
    - `unlock` and `create --unlock`: the address the CLI's own daemon child derived from the
      seed the password opened, reported on that child's channel (both also add `daemonPid`).
      `unlock` also shows that key's derivation `index`, and the same `warning` as `--verify`
      when the index is one zwallet never assigns (checked against the key list it read before
      the prompt).
    - `create`: the first key, derived from the seed generated in the same process, and a later
      key when the password was asked for (see above).
  - `verified: true, verifiedBy: "session"`: while unlocked, `address`, `addresses`, `status`,
    `change` and `balance` (with or without `--chain`) ask the key daemon to derive the address
    (once per chain family), compare it with the cache, and check with the kernel which process
    answered. On **macOS** that check identifies the code actually running, not a path or a value
    the binary declares about itself (both of which a same-user attacker can forge): using the
    peer's audit token (`LOCAL_PEERTOKEN`, so pid reuse cannot swap the target), the peer's
    code-signing hash (`csops` `CS_OPS_CDHASH`, the hash the kernel validates against the mapped
    code pages; every macOS build zwallet ships is signed, see [Install](../README.md#install)) must equal this
    process's own, the peer must be `CS_VALID`, its argv must be in the process table as a key
    daemon of this home, it must hold the listening `d.sock` of this home, and the executable UUID
    it used the socket with (`LOCAL_PEERUUID`) must match. That daemon decrypted this home's
    `wallet.json` with the password typed at `unlock`. On **Linux** every zwallet process is
    non-dumpable, which hides `/proc/<pid>/exe` from its own user, so the daemon's code cannot be
    checked there and nothing is verified by session: the address is shown as below.
  - `verified: false`: the cache while locked, with the line ``not verified: the wallet is locked;
    `zwallet unlock <dur>` or `zwallet address --verify` checks it`` in human output; or, while
    unlocked, an address no session verified (an agent's `create`, `list` without `--verify`, or
    Linux), with ``not verified: run `zwallet address --verify` before funding``. When the
    process answering on `d.sock` is known not to be this build's key daemon of this home
    (another program, another zwallet build such as the one before a `brew upgrade`, or one that
    does not hold the socket), the command still succeeds, unverified, with a `warning` naming the
    pid and what it is, ending ``run `zwallet lock` and unlock again``. A process whose code cannot
    be read (an unsigned program, such as an x86_64 one on any Mac, or a non-dumpable one on
    Linux) is not named: its answer is only not verified. `balance` adds no note line. `list`
    without `--verify` never asks to verify the active key while this build's key daemon of the
    home derived the rows (macOS): its note is then ``not verified: the other keys' addresses;
    `zwallet list --verify` checks them``, and with one key there is none; locked, on Linux, or
    with another process answering, it is the note above.

  While a daemon answers, a cache that its answer contradicts fails these commands (and `list`)
  with `VALIDATION_ERROR`. Only when every process that answered is this build's key daemon (so
  only on macOS) does that error say the cache was modified. Otherwise the answer is no proven
  derivation, so the error says the two addresses disagree, that one of them was tampered with,
  to fund neither and to run `zwallet address --verify`, which decides with the password; a
  process known to be another one is also named, in the message and the `warning` field. An
  answer that is not an address, or a cached value that is not one, is `VALIDATION_ERROR` too and
  is never shown, compared or verified as an address.

  What reaches the terminal, exactly. Every byte zwallet writes for a human (results, errors,
  notes, the whitelist change summary, the password prompt) goes through one write point that
  lets through only printable ASCII (`0x20`–`0x7E`), the newline and `₮` (U+20AE, in the
  registry's `USD₮0`); any other character — `\r`, tab, ESC, DEL, C1, bidi controls, line
  separators, block and box-drawing glyphs, emoji, accented letters — is written as a `\u{…}`
  escape. Two writes do not go through it: zwallet's own QR code of a verified address, printed
  after the text, and clap's help, version, usage and argument errors (`--help`, `--version`, a
  mistyped command line without `--json`), which repeat only the command line the invoker typed
  (no argument is read from the environment). `--json` is printable ASCII too: every other
  character inside a string is a JSON `\uXXXX` escape (two for a character above U+FFFF), so a
  JSON parser reads back the exact string.

  Before that, each source zwallet does not control is escaped where it comes in (the same
  `\u{…}` form, with `\` itself as `\u{5c}`, so the encoding is one-to-one) and bounded. What
  holds, as one invariant (every cut counts its `...(cut)` marker):
  - **whitelist.json and whitelist.next.json** (anyone of this user's to write): a file is a list
    only if every value in it is in the shape zwallet writes — a nickname of `a-z 0-9 _ -`, an
    address or selector of `0x` and hex digits, a chain `*` or a chain key's shape (`a-z 0-9 -`),
    a fee cap of decimal digits — else it is not a list, and the message names the field, never
    its value; the daemon then trusts nothing from that file. So every value from these files a
    human sees is in that alphabet, or `<invalid>` (a chain zwallet does not know, a field of a
    file that is not a list). Two exceptions, each bounded: the parser's message for a file JSON
    cannot read as a list quotes at most 80 characters of it; and `whitelist reset` shows the
    entries of the files it discards as found, at most 512 per file, on a line that holds nothing
    else from the file (`whitelist.json`'s below the line with its reason, which quotes at most
    the parser's 80, or the file's generation, and `whitelist.next.json`'s when it is not the
    sealed list).
  - **Names and fee caps from those files** in one command: at most 512 characters in all, each
    name or cap whole, then how many more (the change summary before the password, with the old
    caps of a `fee-cap` and the caps a `reset` sets back, counted before any name; `whitelist
    list` and its fee caps for a list that did not verify). Each file's names are shown once: a
    `reset`'s summary names them where it says what it drops, and the file's own line gives its
    generation and count only. A list the seal verified (`whitelist list --verify`) is the
    human's own, signed by the key only the password derives: up to 100 entries.
  - **The send journal:** at most 512 characters per message, each field in the shape zwallet
    writes it or `<invalid>` (see **The send journal**).
  - **Everything else from outside**, at most 512 characters per source: an error the process on
    `d.sock` sends (its message and its string or number details together; array and object
    details are dropped); `status`'s whitelist and a sign reply's `signerDecoded` (a value that
    is not an object is one string of at most 512 in `--json` too); an RPC endpoint's error text;
    a decode error's return data and message (128 + 384); a `call` result in human output; and,
    at 64 or less, a relayed `expiresAt`, a token's `symbol()` (32), a cached address or an answer
    that is not an address. Exceptions, each named and bounded: a relayed error from a process
    known to be another one names its executable path (64) beside its message (512); `status`
    shows the serving process's `expiresAt` (64), whitelist (512) and, from another process, its
    path in the warning (200), on three labelled lines; `call`'s human output adds the returned
    bytes as hex (512) on its own line (`--json` keeps the result and the hex whole).

  A QR code holding an Ethereum address needs at least version 2: a block of 25 × 25 = 625
  modules without a quiet zone, 25 rows of 25 cells from one source at one character per module
  (a Tron address, 34 bytes, needs version 3: 29 × 29 = 841).
  A source bounded to 512 characters cannot fill one. The exceptions above are not shown unable
  to: their parts sit on separate labelled lines, or are hex only. What a bounded source can
  still do is pad to the terminal's width so that its text starts a visual row at column 0 (soft
  wrap), and on a terminal exactly 21 columns wide, a 512-character budget fits a version-1 code
  (441 modules, at most 17 bytes: a short link, not an address) with no quiet zone. All of this
  is what a same-user process can already do by writing to the terminal directly (**Known
  limits**), so it is documented, not mitigated.

  Two places keep `₮`, the one non-ASCII character the write point lets through: a registry
  token whose `symbol()` is exactly the symbol the registry records shows it as is (USDT0's
  `USD₮0`; any other symbol, from that address or another, is escaped), and a send journal
  amount whose symbol has it (the shape check lets `₮` into any journaled symbol, so a line
  zwallet did not write can carry up to 32 of them too; in `--json` it is `\u{20ae}`, as every
  non-ASCII character of a file value is). An error from the process on `d.sock` reads as its own:
  prefixed ``the process answering on the key daemon socket reported:`` unless it is this
  build's key daemon, or, from a process known to be another one, named, with its details
  dropped. A `WALLET_LOCKED` reply from any process running this build's code gets the neutral
  prefix instead of being named (a daemon whose session just ended looks like another process by
  then), and still no details: a genuine `WALLET_LOCKED` carries none.
  `wallet.json` must hold only key names `create` accepts (`active` too); a file with another is
  `VALIDATION_ERROR`. What a session cannot rule out: a process of this user that
  swaps in its own `wallet.json` and unlocks it with its own password through this very binary (a
  genuine daemon of a wallet the human did not make), or injects code into the genuine binary
  (`DYLD_INSERT_LIBRARIES` on a binary without the hardened runtime, or on any binary while System
  Integrity Protection is off; `LD_PRELOAD`) — the same
  class as replacing the binary, which nothing here can prevent. So keep the address you saved
  from your first unlock; if it ever changes, stop. **Piped and `--json` output changed with
  this check**: the locked note now names `zwallet unlock`, and a session-verified result gains
  `verifiedBy: "session"` and drops the unverified note; agents should read `--json`.
## Address QR codes

- **QR code**: where the address is `verified: true`, by the password (`unlock`, `create` and
  `create --unlock` with a verified key, `address --verify`) or by the session (`address`,
  `addresses`, `status`, `change` and `balance` while unlocked), human output on a terminal ends
  with the lines
  `scan to send to <address>` and ``(the same address on every EVM network in `zwallet chains`)``,
  then the address as a QR code for a phone wallet: the plain EIP-55 address (no URI scheme, no
  chain), error correction level M, a quiet zone of 4 modules, black on white in ANSI colours, or
  with `NO_COLOR` plain blocks for a dark terminal. A Tron address (`address --chain tron`,
  `balance --chain tron`) has its own code of the plain `T…` under the lines `scan to send to
  T…` and `(Tron only: TRX and TRC20 tokens; not an EVM address)`. Scan that one and save it in the phone
  wallet's address book. The `create` that makes the master seed shows no code, only a line to run
  `zwallet backup` first: funds sent before the recovery phrase is written down are at risk.
  `--no-qr` leaves either out. `addresses` shows two codes, the EVM one (the address is the same
  on every EVM chain) then the Tron one, each under its lines, and `balance` shows the EVM one after
  the networks. Nothing unverified prints one, and neither
  `--json` nor output into a pipe ever has one, so agents see the same text as before.
## The whitelist

- **The whitelist** ([`.prd/04-whitelist.md`](../.prd/04-whitelist.md)): the key daemon signs a
  transaction that moves value only when a list signed with this wallet's whitelist key allows
  it. That ed25519 key is not derived from the seed, and it is never stored: every whitelist
  change derives it again from the password you type and a random per-wallet salt sealed in
  `wallet.json` (its own scrypt run, with the wallet's parameters). After `unlock` the daemon
  keeps only the seed and, from the sealed policy, the key's public half, the list's generation
  and its digest (the rest of the decrypted policy, the salt included, is dropped once the session
  starts), so the daemon can check a list but never sign one. The generation and the digest are
  there because the public key alone would also accept an older list the same key signed. The
  seed is not derived from the password: it is a random BIP39 mnemonic encrypted under it, since
  a password-derived seed would be a "brain wallet" that anyone could attack by guessing
  passwords against its public addresses. The list (`whitelist.json`) is plain JSON anyone can
  read; the daemon trusts it only when it verifies.
  - **Entries.** `whitelist add <nick> <0x>` lists a payee: native coin and registry tokens
    (USDC, USDT, DAI and WETH where their issuers publish them) may be sent to it on its
    `--chains` (comma-separated chain keys, default `ethereum`; `'*'`, quoted, for every chain,
    only when given). `whitelist add <nick> <0x> --chains <c> --contract --selectors
    <0x12345678,…>` lists a contract that may be called with those 4-byte selectors only (named
    chains, never `*` and never a registry token); it prints `warning: a listed contract may
    forward value or act on its arguments`. Nicknames are 1-32 of `a-z 0-9 _ -` starting with a
    letter or digit, and each address is listed once.
  - **What the daemon allows**, checked on every signature against the list it verifies right
    then. A native transfer goes to a payee on one of its chains, or to the signing key's own
    address (with a list that verifies, or while the wallet has no whitelist yet; see **Fees**).
    Calldata is judged by what the daemon decodes:
    - **ERC-20 `transfer`, `approve`, `transferFrom`** (decoded from their canonical encoding,
      whatever the entries say): a `transfer` only of a registry token on that chain, to a
      payee on that chain that is not itself a registry token on any chain (a token transfer to
      the key's own address needs a payee entry too); an `approve` only of a registry token on
      that chain, and only of 0 (a revoke; any spender); `transferFrom` never. The same calls on
      a token outside the registry are refused.
    - **Any other call** goes only to a contract entry on one of its chains, with one of its
      listed selectors (native value only then), and never to a registry token. A contract
      entry delegates: the agent may call its listed selectors with arbitrary arguments;
      configured native-value and calldata limits still apply. The contract may forward value
      or act on those arguments. That holds for a token outside the registry
      listed as a contract, too: its listed selectors (say `increaseAllowance`) are called like
      any listed contract's.

    Everything else is `WHITELIST_DENIED`, and so is `sign-message` until `whitelist
    allow-message-signing on`.
  - **Fees.** Each signature, a native self-transfer included, must pass the execution-fee cap
    for its chain: its fee limit (`gasLimit × maxFeePerGas`, or `× gasPrice`) against the
    verified list's cap, or the built-in default before any whitelist exists (no anchor
    sealed); otherwise `FEE_CAP_EXCEEDED`. With a verified list the action is decided first, so
    an action the list refuses is `WHITELIST_DENIED` whatever its fee, and a fee refusal only
    ever concerns a transaction the list would otherwise sign. A list with no readable cap for
    the chain, or a chain with no default, is refused with the `whitelist fee-cap` command. The
    cap bounds each transaction, not a total: repeated transactions can each spend up to it. On
    the OP-stack chains (Base, Optimism) the L1 data fee is charged on top of it. The defaults
    are ethereum 0.003 ETH; base, arbitrum and optimism 0.0005 ETH; polygon 2 POL; bsc 0.003
    BNB; avalanche 0.05 AVAX; sepolia 0.01 ETH; tron 30 TRX (signed fee_limit only),
    changed with `whitelist fee-cap <chain>
    <amount>`.
  - **Optional transaction limits.** A verified list can also cap decoded native value,
    direct registry-token amounts and calldata size per chain. Exceeding one is
    `WHITELIST_DENIED`; no signature is produced. See [per-transaction limits](#per-transaction-limits)
    for units, configuration and limits of enforcement.
  - **A list that cannot be trusted** (it does not verify: edited by hand, an older copy; it
    cannot be read, or not within the 2 s deadline; or the sealed anchor is malformed) refuses
    **every** transaction with `WHITELIST_INVALID`, a native self-transfer included: a broken
    list never relaxes a stricter signed fee cap. The message gives the reason and points to
    `zwallet whitelist list --verify`, with no `command`: an edit suggested from the refused
    transaction would name an address an agent chose. A `whitelist.json` above the sealed
    generation gets its own reason, by its generation and signer (see [Joint rollback](security.md#known-limits-same-user)). Two or more edits above, under the sealed key: "wallet.json was rolled back".
    One edit above: an edit that never finished sealing, or a rolled-back `wallet.json`.
    Another key: a rollback across a reset, a reset that never finished sealing, or a foreign
    file. A wallet with **no whitelist
    yet** signs only a native self-transfer under the built-in cap; a self-transfer above it, or on
    a chain with no built-in cap, is `FEE_CAP_EXCEEDED` with the `whitelist fee-cap` command, and
    anything else is `WHITELIST_INVALID`.
  - **What a refusal says.**
    - A transaction refusal (`WHITELIST_DENIED`, `WHITELIST_INVALID`, `FEE_CAP_EXCEEDED`)
      carries `counterparty` and `chain`, and `command` with the exact command when one edit of
      the list would allow what was refused: `zwallet whitelist add <nick> <addr> --chains
      <chain>` for an address not in the list (for a contract, with `--contract --selectors
      <selector>`), or `zwallet whitelist fee-cap <chain> <this transaction's fee>`. A fee
      refusal also carries `feeWei`, and `capWei` when there is a cap. A `fee-cap` command
      comes only with an action the list allows, so it is the one edit that lets the request
      through; after an add, a request over the cap too is refused next with its own `fee-cap`
      command.
    - `command` is left out when no edit can allow it (`transferFrom`, an approve of more than 0
      or on a token outside the registry, another call on a registry token, a token transfer to
      a token contract, an address listed as the other kind); when an entry exists but does not
      cover the chain or the selector (the message names the entry and what it is missing); and
      for every refusal of a list that cannot be trusted. With no whitelist yet, `command` is the
      first `whitelist add` only when an entry could allow the action: a forbidden action has
      none, and neither has a revoke of a registry token (any signed list allows it).
    - A `sign-message` refusal carries no `counterparty` or `chain`: `WHITELIST_DENIED` with
      `command: zwallet whitelist allow-message-signing on` while the verified list has it off;
      `WHITELIST_INVALID` with that `command` while there is no whitelist; `WHITELIST_INVALID`
      with no `command`, pointing to `list --verify`, while the list cannot be trusted.
  - **`whitelist list`** works while locked: `whitelist.json` as it is, `verified: false`. Human
    output shows a field of an entry as it is only when it passes the check `add` applies to it
    (the nickname rule, `address` or `contract`, `0x` and 40 hex digits, a known chain key or
    `*`, `0x` and 8 hex digits), else `<invalid>`; `gen` is a number or `<invalid>`; at most 16
    chains, selectors and fee caps each. A list that is not `verified: true` shows its entry lines
    and fee caps within one 512-character budget (about 5 entries), then "... N more; `zwallet
    whitelist list --verify` shows them all" (after a `--verify` that did not verify, `--json`
    does); a file that cannot be read as a list is also counted. A verified list (`--verify`)
    shows up to 100 entries. `--json` keeps every entry, its strings escaped.
    `--verify` (password) checks it against the seal: `verified: true` with
    `source` and `sealedGen`, or `verified: false` with the `reason` (with no whitelist sealed
    but a list file there, why every edit refuses and what to do, not the first `add`; for a
    `whitelist.json` that does not verify and is not above the seal (deleted, unreadable, not a
    list, edited, or older), the same one recovery as an edit's). Plain `list`
    with only `whitelist.next.json` there names it and points to `--verify`. `unlock`, `create
    --unlock` and `status` show what the daemon trusts, read again on every `status`: `whitelist: {
    gen, verified: true, entries: [nicknames] }`, or `{ gen (null without a whitelist), verified:
    false, entries: [], reason }`.
  - **Changes** (`add`, `remove`, `fee-cap`, `native-cap`, `token-cap`, `calldata-cap`, `allow-message-signing on|off`, `reset`) ask for the
    password (terminal prompt, or `--password-stdin`). Before the password is read,
    `whitelist.json` and `whitelist.next.json` are read once and the change is printed where the
    password is read from (the terminal, or stderr with `--password-stdin`): the nickname, the
    full address, payee or contract, the chains (`*` as "every chain", otherwise zwallet's key for
    each or `<invalid>`, at most 16), the selectors, a policy change's old and new value (from
    both files when an unfinished edit is in `whitelist.next.json`), what a `reset` drops and
    every fee cap it sets back (by zwallet's key for each chain, once; of the files' caps that
    differ, each that still fits the budget is shown beside it, then "and N more caps from the
    files, not shown"), both files as they are (at most 20 names per file, on a `reset` only
    where it says what it drops; names and fee caps from the files share one budget of 512
    characters, the caps first and each file's names once, then how many more; a file that
    exists but cannot be read, or is not a list, is "there but cannot be read: its entries are
    unknown, and a reset drops them", never "none"), every outcome
    that can still happen (for an edit: the change lands on `whitelist.json` if it is the sealed
    list, on the unfinished edit if that is, on a first list from the defaults if there is no
    list file and no whitelist yet, otherwise nothing changes; for a `reset`: the generation it
    makes in each of those cases, "if neither file is the sealed list (it was changed or
    deleted): gen M+1, where M is the sealed generation", and gen 1 with no whitelist sealed),
    that every zwallet daemon of this user is
    stopped, and what happens to signing: "after the
    change: the wallet stays locked", or with `--unlock <duration>` "after the change: a new
    signing session for <duration> (until about <local time>, <UTC>)". The last line, right above
    the prompt, repeats the change and what happens to signing. **The change is applied to
    exactly the list shown, or nothing changes**: the files are checked again right before
    anything is stopped, and read again once under `w.lock`; if they are not those bytes, the
    change is refused ("<file> changed after it was shown; nothing in the whitelist changed").
    An edit builds only on the list `wallet.json` seals: if the list shown does not verify
    (edited by hand, an older copy, deleted, or newer than a rolled-back `wallet.json`), or
    `wallet.json` seals no whitelist but a list file exists (a `wallet.json` restored from
    before the first edit, a planted file, or a first edit interrupted before its re-seal, which
    leaves only `whitelist.next.json` at gen 1), nothing in the whitelist changes, and the
    message points to `zwallet whitelist list --verify`. Whatever the cause, it gives one
    recovery: "If wallet.json is an older copy, restore the current one if you have it (zwallet
    keeps no copy of it); otherwise `zwallet whitelist reset` …". zwallet never suggests restoring
    an older `whitelist.json` (see [Known limits](security.md#known-limits-same-user)), so recovering from accidental damage to the
    list means restoring `wallet.json` if an older one went back, or a reset. `zwallet whitelist
    reset` starts over, at a cost the summary and the message spell out: every fee cap goes back to
    the built-in default, all optional transaction limits are removed, and no payee or contract
    is listed (native self-transfers under the default fee caps and approve-0 revokes still sign).
    Reset warns about removed transaction limits even when the old list cannot be read.
    Configure fee caps and transaction limits again before `--unlock`. A wrong password stops
    nothing; neither does a list an edit cannot build on (an edit; a `reset` goes ahead), or a
    change the list refuses. Otherwise the change stops every
    zwallet daemon of this user (as `lock` does), signs the next generation and re-seals it into
    `wallet.json`, and the wallet stays locked unless `--unlock <duration>` starts a new session for
    that duration with the password just typed (with `unlock`'s checks, so on Linux it may need
    `--allow-ptrace-scope-0`). Output: `{ change, gen, previousGen, base, entries: [nicknames],
    verified: true, stoppedDaemons, discarded?, discardedNext?, warning?, locked, hint? }` plus the
    change's own fields, and with `--unlock` `{ duration, expiresAt, daemonPid, whitelist }` (or
    `unlockError`). Every change derives the whitelist key from the password just typed and the
    sealed salt, and refuses, writing nothing, if that is not the key `wallet.json` seals ("the
    password-derived whitelist key is not the one wallet.json seals; …"). The first `add` draws the
    salt, so the key. `reset` draws a new salt, so a new key, and makes an empty list with the
    default caps, for when the list does not verify and the current `wallet.json` is in place (or
    there is no copy of it), or for a whitelist made by zwallet 0.1.0 (or a `wallet.json` restored
    from before the upgrade), which every other change refuses before anything is stopped ("this
    whitelist was made by zwallet 0.1.0, whose key file is no longer used …", with the one
    recovery); the daemon signs nothing under it, and its reason names that cause alone; when the
    list it replaces did not verify (or
    `wallet.json` sealed none), `discarded` shows it (the reason and its unverified entries, as
    JSON, `null` when the file is missing, cannot be read or has no entries; the human output then
    says "no verified entry; whitelist.json's entries unknown" where it says what was dropped), and
    `discardedNext` a dropped `whitelist.next.json` that was not the sealed list. If
    `whitelist.next.json` cannot be removed once the new list is published (or the directory not
    synced after), the change still succeeds, with a `warning` that says to remove it (a later
    change can stage its list only where there is a plain file or nothing), and `--unlock` still
    runs; the next summary shows the file as "a copy of whitelist.json (e.g. left after a
    change)". An edit never has `discarded`: it refuses instead.
  - **`--dry-run`**, when there is a `whitelist.json`, first runs the daemon's own check on it as
    it is on disk (unverified; one that cannot be read counts as untrusted). If that refuses, the
    dry-run reports `whitelist: { allowed: false, reason, code, command?, counterparty, chain, …
    }` and asks the daemon for no signature (no `txHash`, no `recoveredFrom`). If it allows, the
    daemon checks and signs as for a live send, and the output adds `whitelist: { allowed: true
    }`. Without a `whitelist.json` the CLI checks nothing (it cannot tell a wallet with no
    whitelist yet from one whose list was deleted; the anchor is sealed): the daemon decides, its
    refusal is the same error as a live send's, and there is no `whitelist` field. A live send is
    decided by the daemon alone.
  - **`@nick`**: `transfer --to @alice` and `invoke --to @router` take the address from
    `whitelist.json` as it is, unverified (the daemon decides anyway), and add `toNickname`. An
    unknown nickname is `VALIDATION_ERROR` naming `zwallet whitelist list`.

## Per-transaction limits

A human can add optional limits to the signed whitelist. These commands use the existing password
prompt, end unlocked sessions, and support `--unlock <duration>` like other whitelist edits:

```sh
zwallet whitelist native-cap ethereum 0.01
zwallet whitelist token-cap ethereum 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48 1000000
zwallet whitelist calldata-cap ethereum 4096
zwallet whitelist list --verify
```

`native-cap` takes decimal native coin units (0.01 ETH above); `token-cap` takes **integer smallest
token units** (1,000,000 = 1 USDC above), and accepts only a registry deployment on that chain.
`calldata-cap` takes a byte count. Zero forbids positive value/amount or nonempty calldata;
`none` removes that specific limit. Limits apply per chain to all keys/payees, and the native
limit also applies to self-transfers and native value attached to allowed contract calls.
The daemon enforces the limits on the decoded bytes it signs; hand-editing the file cannot raise them.

Old lists without limits retain their original signatures and behavior. Old binaries reject lists
containing the new fields, so keep this version when limits are configured. `whitelist reset`
removes these limits and warns before the password prompt; configure them again before unlocking.
The `--json` policy field `transactionLimits` is keyed by chain and exposes optional
`maxNativeWei` as a decimal integer string, `maxTokenBaseUnits` as a map from token
addresses to decimal integer strings, and `maxCalldataBytes` as an integer. It is omitted when
no limits are configured.

These limits do **not** enforce a cumulative budget, restrict arbitrary contract argument effects,
constrain authority granted by message signatures, or cap OP-stack L1 data fees. A calldata limit
only bounds payload size. Strong cumulative budgets and joint-rollback protection remain design
issues [#26](https://github.com/2lab-ai/zwallet/issues/26) and
[#27](https://github.com/2lab-ai/zwallet/issues/27); this implementation adds no external trust store.

## Balances

- **`balance` without `--chain`** checks every network in [Chains](../README.md#chains), mainnets first and the
  testnet (Sepolia) last: the native coin and every registry token there (USDC, USDT, DAI and
  WETH where their issuers publish them; BNB Smart Chain has none), zero balances included in
  `--json`. Token symbols and decimals come from the registry; `--token` instead asks the
  contract. The networks are read at once, each through `ZWALLET_RPC_<KEY>` or its default RPC
  and within 8 s for all its reads; `--token` and `--rpc` need `--chain` (a contract and an RPC
  URL belong to one chain). A network whose endpoint, chain id check or native balance fails or
  runs out of time has `error` and `errorCode` (and no `native` or `tokens`); a token whose
  `balanceOf` does has them on its own row (and no `balance` or `raw`), and the rest of the
  network still shows. The command exits 0 when any network answered; when none did it fails with
  their shared error code (else `RPC_ERROR`) and every network's error in the message, each cut
  short to 72 characters as its line shows it; `--json` adds `networks: [{ chain, error,
  errorCode }]`, each error whole as a network's row has it (human output never shows it). The
  address is the same on every EVM chain; a row has its own `address` only where the cache
  differs, as the `tron` row always does. The `tron` row is TRX and USDT (the Tron registry's
  only token), with `activated` and, when the account's owner permission is not this key alone
  at threshold 1, its own `warning` (also a line under it). Human output: `key` and `address`, one line per network with its coin, indented under
  it each non-zero token and each failed token as `<SYMBOL> error: …`, a failed network as
  `error: <message, cut short> [<CODE>]`, and a last line `<n> of <all> networks have a balance`
  (`; <m> did not answer`, `; <k> token reads failed` when some failed).
- **`balance --chain tron`** checks the endpoint first: block 0's id (`/wallet/getblockbynum`)
  must be mainnet's genesis id, else `RPC_ERROR` naming both, before any account is read. It
  then reads `/wallet/getaccount`: TRX in sun (6 decimals), `activated: false` when the node does
  not know the account (it never received TRX), and a `warning` when the account's owner
  permission is not this address alone at threshold 1 ("funds here may not be controlled by
  this wallet's key; do not fund it"; the keys it names are escaped and cut). `--token usdt` (any
  case) reads USDT's `balanceOf` with the registry's symbol and decimals (`symbolSource:
  "registry"`); `--token <T…>` asks the contract for `decimals()` and `symbol()`, shown escaped
  (`symbolSource: "contract"` unless it is the registry contract answering its recorded
  symbol). Only one 32-byte word with `result.result: true` is read as a balance; a balance that
  is not a whole number of sun below 2^63, an answer about another account, or a failed call is
  `RPC_ERROR` (the node's message escaped and cut at 512), never shown as a balance. Requests
  carry `User-Agent: zwallet/<version>` and nothing about who runs them.
## Contract calls

- **`call` / `invoke` signatures** are `name(argTypes)(returnTypes)`, e.g.
  `balanceOf(address)(uint256)` or `getReserves()(uint112,uint112,uint32)`. Return types are
  optional (without them `result` is null and `raw` has the hex). Arrays are written `[1,2]`,
  tuples `(a,b)`; scalar results are strings. `uint`/`int` mean `uint256`/`int256` (the selector
  is computed from the canonical names). Arguments:
  - `address`: `0x` plus 40 hex digits; mixed case must be a valid EIP-55 checksum.
  - integers: plain decimal (`-` allowed for `int`) or `0x` hex; no units or exponents.
  - `bytes`, `bytesN`, `function`: `0x` hex, exactly N bytes for `bytesN` (24 for `function`).
  - `bool`: `true` or `false`.
  - a top-level `string` argument is taken exactly as given (no trimming, quotes included).
  - inside a list, every element is trimmed. A string element that starts with `"` or `'` must
    end with the same quote, which is removed; quoting keeps commas, brackets and surrounding
    spaces. There are no escapes: to include one kind of quote, quote with the other
    (`["a, b", 'say "hi"', plain words]`).
## Transfers and transaction results

- **`transfer` / `invoke`** fill the pending nonce, EIP-1559 fees (legacy `gasPrice` where the chain
  has no base fee), `eth_estimateGas` + 20% (exactly 21000 for plain transfers) and, on Base and OP
  Mainnet, the L1 data fee. Without `--dry-run` a shortfall is `INSUFFICIENT_FUNDS` before anything
  is signed. `--dry-run` signs but does not broadcast and returns `{ dryRun: true, chain, key, from,
  to, amount, amountRaw, symbol, decimals, token?, chainId, nonce, pendingNonces, pendingRange,
  pendingCount, duplicateSends?, gasLimit, gasEstimate, gasLimitSource, type, maxFeePerGas,
  maxPriorityFeePerGas, gasPrice, maxGasCost, l1Fee, nativeBalance, sufficientFunds, shortfall,
  whitelist, signerDecoded, txHash, recoveredFrom }` (without the last three when the whitelist
  refuses, see **The whitelist**). It has no `rawTx`: a dry-run never hands out bytes that
  could be broadcast without zwallet. `recoveredFrom` is recovered from the signed raw bytes,
  `signerDecoded` is what the key daemon decoded from the bytes it signed, and an empty wallet can
  be dry-run. `pendingNonces` lists the key's nonces that are pending (between its mined count and
  its next free nonce) on the node, at most the first 64; `pendingRange` is `[first, last]` of them
  (or `null`) and `pendingCount` how many there are. Both counts come from the node, so the range is
  never listed in full. `duplicateSends` (a send without `--nonce`) lists the journal's recent sends
  that this one repeats (see below). Live: `{ ..., txHash, explorerUrl, journal, broadcastWarning?,
  rpcTxHash? }` plus `{ status, success, blockNumber, gasUsed, hint? }` with `--wait`.
- **`--wait`** polls for the receipt for `--wait-timeout <dur>` (default `90s`; same units as
  unlock durations). One deadline bounds both in-flight receipt RPC requests and polling sleeps;
  a late response is not accepted after expiry. This budget starts after broadcast and does not
  bound transaction preparation or broadcasting. A receipt is `status: "success"` (`success: true`)
  when its status is exactly `0x1`, `"reverted"` (`success: false`) when it is exactly `0x0`, and `"mined"` with
  `success: null` otherwise (no status, as before Byzantium, or any other value): the execution
  outcome is unknown, never a failure, and a `hint` says "do not send this payment again; check
  the receipt on the explorer" with the explorer page. Not mined by then is `status: "pending"`,
  and a receipt RPC problem is `status: "unknown"` with `receiptError`; neither turns a broadcast
  into an error, and both add ``hint: "still pending; to speed up or cancel use --nonce N
  --replace <txHash>; a run without --nonce is a second payment; look it up with `zwallet tx
  --chain <chain> <txHash>` or <explorer>/tx/<txHash>"`` (for `unknown` it starts "receipt
  unknown, it may still be pending").
- **`tx --chain <key> <txHash>`** looks a transaction up on the chain's node
  (`eth_getTransactionByHash` and `eth_getTransactionReceipt`). It needs no wallet and no key, so
  it works while locked. `status` is `"mined"` (with `blockNumber`, and `gasUsed` and `success`
  from the receipt: `true` for status `0x1`, `false` for `0x0`, `null` with the same `hint` as
  `--wait` for anything else, and `null` with a hint that this node has no receipt when the
  transaction names its block but the node returns none), `"pending"`, or `"not found"` (`found: false`, and `from`, `to`,
  `nonce`, `value` are `null`). `value` is in the chain's coin (`valueRaw` in wei); for a token
  transfer it is `0` and `to` is the token contract. A pending transaction adds `hint: "still
  pending; to speed up or cancel use --nonce N --replace <txHash>; a run without --nonce is a
  second payment"`. Not found adds a `hint` that this does not mean it was never sent (it may still
  reach the network, be held by other nodes, or have been dropped) and that a retry of the
  payment passes `--nonce <the nonce it was signed with>`, never a plain resend. A node that
  answers about another transaction is `RPC_ERROR`, never "not found". Every message below that
  says to look a transaction up names this command and the explorer page: with the hash for a
  transaction zwallet itself sent or found mined, and for the send journal's entries their
  hashes, then one `zwallet tx --chain <chain> <txHash>` or `/tx/<txHash>` template.
## Send journal and retry safety

- **The send journal.** Right before `eth_sendRawTransaction`, zwallet appends one JSON line
  `{ ts, chain, from, nonce, txHash, to, value, amountRaw, kind, token? }` (`value` like
  `"0.01 ETH"` for people, `amountRaw` the same amount in base units: wei, or the token's smallest
  unit; `kind` `transfer`, `token-transfer` or `invoke`, `to` the recipient or the contract,
  `token` the token contract of a token transfer) to
  `sends-<chainId>-<address>.jsonl` in the wallet home (0600, fsynced), and prints the same fields
  on one stderr line: "broadcasting <txHash> (nonce N on <chain>); if this command does not finish,
  look this hash up before sending again (`zwallet tx --chain <chain> <txHash>` or
  <explorer>/tx/<txHash>); a retry must pass --nonce N --replace <txHash>. Journal <path>: {…}".
  So a run that is killed after the broadcast still leaves its hash behind. If the line cannot be
  written, nothing is broadcast. A record cut off by a killed run (no final newline) gets a
  newline before the next one, so it never swallows that line. The journal is opened without
  following a symlink and without blocking (a FIFO cannot hang a send), and must be a regular
  file owned by this user; otherwise a send without `--nonce` is `VALIDATION_ERROR` "cannot read
  the send journal <path>: …; nothing was signed", since the repeat check below needs it. Errors
  and messages below that send the agent to an earlier transaction name the journal's entries at
  that nonce, then how to look each hash up (`zwallet tx --chain <chain> <txHash>` or the
  explorer's `/tx/<txHash>` page). The journal is a file any process of this user can write, so
  a message shows each field of an entry only in the shape zwallet writes it, else `<invalid>`:
  the time as RFC 3339 UTC (re-rendered from the instant, not the file's text), the kind
  `transfer`, `token-transfer` or `invoke`, the recipient `0x` and 40 hex digits, the amount as
  `format_units` writes it (at most 78 digits before a point and 255 after, 257 characters in
  all), one space and the symbol zwallet journaled (1 to 32 printable characters other than a
  space, `₮` included: the chain's, a registry token's, `?` when `symbol()` failed, or an
  on-chain one as `erc20_meta` escaped and cut it; one with a space in it shows as `<invalid>`),
  and the hash only when it is a 32-byte hash (`?` else). The journal's text in one message is at
  most 512 characters (an entry is at most 452, so the newest always fits): of the 5 newest
  entries, newest first, each that still fits, shown whole and oldest first (one that does not
  fit is skipped, so it cannot hide a smaller one behind it), then "(and N more there)" for every
  entry not shown. The human `duplicateSends` line of a dry-run or an `--allow-duplicate`
  send shows its entries the same way. The entries in `duplicateSends`, `pendingSends` and
  `journalEntries` (on the other messages that name the journal's entries) keep all their fields,
  every string escaped.
- **One send at a time.** Sends from one key on one chain never overlap: each holds
  `nonce-<chainId>-<address>.lock` in the wallet home from before reading the nonce until the
  command returns, through `--wait`. A send that finds the lock held does not wait (after waiting,
  a second attempt at the same payment would sign the next nonce and land too): it fails at once
  with `VALIDATION_ERROR`, before any RPC call, so nothing is signed or broadcast (see **Error
  codes**). The message says where the running send's txHash will be (its output, and the send
  journal once it is broadcast) and how to look it up. A dry-run takes no lock.
- **A send without `--nonce` while one is pending is refused.** If the key has a transaction pending
  on the node (its next free nonce is above its mined count), a send without `--nonce` is
  `VALIDATION_ERROR` "a transaction from this key is still pending on <chain> at nonce N; nothing
  was signed", with the journal's entries at those nonces (`pendingNonces`, `pendingRange`,
  `pendingCount`, `pendingSends`, `journal`). The usual cause is a re-run of a send that was cut
  off, which would land at the next nonce as a second payment: look the pending transaction up, and
  to speed it up or replace it use `--nonce N --replace <its txHash>`. `--allow-pending` sends
  anyway; use it only for a different payment. A dry-run reports `pendingNonces` instead of
  refusing.
- **A send without `--nonce` that repeats a recent one is refused.** If the send journal has an
  entry from the last 10 minutes with the same chain, kind, `to` and amount in base units
  (`amountRaw`: wei for a native transfer or an invoke's value; for a token transfer the token's
  base units and the same token contract), a send without `--nonce` is `VALIDATION_ERROR` "this
  looks like a repeat of a recent send; look those hashes up first. If this really is a second
  payment, pass --allow-duplicate", naming each match's txHash, nonce and time, then one `zwallet
  tx` template to look each hash up (`duplicateSends`, `journal`); nothing is signed. The display text (`value`) is
  never compared: a token's symbol comes from the node, and a `symbol()` call that fails once
  (shown as `?`) must not make a repeat look new. A journal line written before `amountRaw`
  existed is compared by its `value`. This catches the re-run of a send that was cut off after it
  was already mined, which the pending check cannot see. A send that was refused or never got out is
  in the journal too, so the lookup comes first there as well. `--allow-duplicate` sends anyway;
  a dry-run reports `duplicateSends` instead of refusing, and a live send without `--nonce`
  reports it too. `--nonce` sends skip the check: at a fixed nonce they can only replace or retry,
  never add a payment. Only the journal's last 1 MiB is read, and lines that are not entries are
  skipped.
- **`signerDecoded.action`** names the ABI method the calldata calls, never an asset, because
  ERC-20 and ERC-721 share these selectors: `{ kind: "native-transfer", to, value }`,
  `{ kind: "transfer-call", token, to, amount, assetType: "unverified" }`,
  `{ kind: "approve-call", contract, spender, value, assetType: "unverified" }`,
  `{ kind: "transfer-from-call", contract, from, to, value, assetType: "unverified" }`, or
  `{ kind: "contract-call", contract, selector, value }`. Amounts are raw base units.
- **`--nonce <n>`** signs with nonce `n` instead of the next free one; `n` must be between the
  account's mined count and its next free nonce (`VALIDATION_ERROR` otherwise). A mined `n` is "the
  transaction at nonce N is already mined (nonce consumed); whether the payment succeeded is not
  known from this alone: look up its receipt with `zwallet tx --chain <chain> <hash>` before
  sending anything new; nothing was signed", with the journal's entries at `n`. Mined only means
  the nonce was used: the transaction there may have reverted and paid nothing, so no message
  says a payment went through from that alone. Where the transaction mined at `n` is known (the
  `--replace` hash or a journal entry at `n` that the node reports as this key's at `n`, with a
  receipt), the error names it (`minedTxHash`, and the hash in the message: 66 hex characters,
  the node's own transaction, shown up to three times beside the journal's 512) and adds the
  receipt's `success` (`true`, `false`, or `null` when its status says nothing); otherwise the
  message has `<its txHash>`. `n` equal to the next free nonce takes that free slot. Below it,
  `--nonce` replaces the transaction pending at `n`, and `--replace <txHash>` must name it (it must
  be this key's, at nonce `n`, not mined); without it the send is `VALIDATION_ERROR` "nonce N
  belongs to a pending transaction; pass --replace <its txHash> to replace it (see the send
  journal)", with the journal's entries at `n`. That message, the mined-nonce one and the
  unknown-`--replace` one below end with "If the transaction at nonce N is a different payment than
  the one you are retrying, do not replace it; that payment and yours are separate". A `--replace`
  hash that is already mined is refused with the same "already mined (nonce consumed)" text, its
  `zwallet tx` command and explorer page, and, when its receipt can be read, `minedTxHash` and
  `success`. zwallet
  reads the replaced transaction's fees from the node and pays
  `--bump` times the higher of them and its own estimate (tip and max fee, or `gasPrice`; default
  1.25, allowed 1.1 to 3) so that nodes accept the replacement. An explicit `--bump` also applies
  when nothing is replaced. If the node does not know the `--replace` hash: where it holds another
  pending transaction at `n`, the error says the hash is the wrong one and names the journal's
  entries at `n`; where nothing is pending at `n`, it says to retry with `--nonce n` and no
  `--replace`. With `--nonce` the output adds `replacesPending` and `feeBump` (e.g. `"1.25x"`, or
  `null` without a bump), and `--replace` adds `replacesTx`.
- **`--max-fee-gwei <gwei>`**: fees come from the RPC node, so zwallet refuses a tip above 2x and a
  price per gas above 6x of max(base fee, `eth_gasPrice`, 1 gwei) with `VALIDATION_ERROR` before
  signing. On a legacy chain (no base fee, e.g. BSC) the price is `eth_gasPrice` itself, so it is
  left out of its own ceiling: `gasPrice` may be at most 6x of max(base fee, 1 gwei), i.e. 6 gwei.
  `--max-fee-gwei` replaces these ceilings with one explicit price per gas (higher or lower); it
  is checked after the bump.
- **`--force`** (`transfer`) allows a recipient that is almost always a mistake: the zero address,
  or the token contract itself. Without it they are `INVALID_ADDRESS`.
- **Amounts** are decimal strings converted exactly (no floats); more fractional digits than the
  asset's decimals is `VALIDATION_ERROR`. ENS names are refused (`INVALID_ADDRESS`).
## Error codes

- **Error codes**: `VALIDATION_ERROR`, `WALLET_EXISTS`, `WALLET_NOT_FOUND`, `WRONG_PASSWORD`,
  `WALLET_LOCKED`, `KEY_EXISTS`, `KEY_NOT_FOUND`, `CHAIN_UNSUPPORTED`, `CAPABILITY_UNSUPPORTED`,
  `INVALID_ADDRESS`, `RPC_ERROR`, `INSUFFICIENT_FUNDS`, `BROADCAST_UNCERTAIN`,
  `WHITELIST_DENIED`, `WHITELIST_INVALID`, `FEE_CAP_EXCEEDED`. Errors from the broadcast step
  carry the signed transaction's `txHash` (or `refusedTxHash`, below), `nonce`, `from` and, once
  the journal line is written, `journal`. zwallet never suggests a plain resend:
  a retry of a send that may have gone out passes `--nonce <nonce> --replace <txHash>`, so it can
  only replace the first attempt, never add a second one.
  - `WHITELIST_DENIED`, `WHITELIST_INVALID`, `FEE_CAP_EXCEEDED` come from the key daemon before it
    signs (see **The whitelist**): nothing was signed or broadcast. They carry `counterparty` and
    `chain`, `feeWei` and `capWei` for a fee, and `command`, the `zwallet whitelist …` command a
    human would run to allow it, when one edit can. Human output prints `counterparty`, `chain`
    and `command` under the message.
  - A failure after signing but before the send request keeps its code (`RPC_ERROR` when the node
    is unreachable) and says "...; nothing was broadcast. If you retry, pass --nonce N ...":
    nothing went out, so nonce N is still free and there is nothing to name with `--replace`.
  - `BROADCAST_UNCERTAIN`: the send failed or timed out, or the node answered with another hash
    (`rpcTxHash`), and the node does not report the transaction by its own hash. It may still be
    on the network. Look it up first (the message names `zwallet tx --chain <chain> <txHash>`
    and the explorer page), or retry with `--nonce <nonce> --replace <txHash>` so the retry
    replaces it instead of sending twice.
  - "nonce N is taken by another transaction from this key, pending or mined": the node answered
    "nonce too low" or "replacement transaction underpriced" and does not report this transaction
    by its hash. This transaction was refused and will never be found, so its hash is
    `refusedTxHash`, not `txHash`, and the message names the journal's other entries at nonce N.
    Without `--nonce` it is `RPC_ERROR`: "If you were retrying an earlier send, look up that
    send's txHash before sending again". With `--nonce` (a retry) it is `BROADCAST_UNCERTAIN`:
    "This retry was refused and will not be found; look up the txHash of your earlier send at
    nonce N from its own output (or the send journal below). While it may be pending or mined, do
    not send this payment without --nonce; to replace it use --nonce N --replace <earlier
    txHash>" (for "underpriced", with a larger `--bump`).
  - `RPC_ERROR` "the node rejected the transaction; nothing was broadcast" when the node refuses
    it outright ("insufficient funds", "intrinsic gas too low", "exceeds block gas limit", "max
    fee per gas less than block base fee", or "transaction underpriced", a price below the pool's
    minimum); it also says to pass `--nonce N` on a retry, and for "transaction underpriced" to
    pay more with `--nonce N --bump <factor>`.
  - A send error or another hash for a transaction the node does report by its own hash (e.g.
    "already known", or "nonce too low" for this very transaction) is a success with
    `broadcastWarning` (and `rpcTxHash` if the node answered another hash).
  - `VALIDATION_ERROR` "another send from this key on <chain> is in progress (lock <path>)":
    nothing was signed or broadcast. If the running send is an earlier attempt at this same
    payment, wait for it to finish and look up its result before sending again (its txHash is in
    its output and, once broadcast, in the send journal; `zwallet tx --chain <chain> <txHash>`);
    for a different payment, retry after it finishes.
  - `VALIDATION_ERROR` "this looks like a repeat of a recent send" and "a transaction from this
    key is still pending on <chain> at nonce N" (a send without `--nonce`, see above) and "nonce N belongs to a pending transaction; pass
    --replace <its txHash>" (`--nonce` below the next free nonce): nothing was signed.

## RPC configuration

| key | chainId | symbol | default RPC |
|---|---|---|---|
| ethereum | 1 | ETH | https://ethereum-rpc.publicnode.com |
| base | 8453 | ETH | https://base-rpc.publicnode.com |
| arbitrum | 42161 | ETH | https://arbitrum-one-rpc.publicnode.com |
| optimism | 10 | ETH | https://optimism-rpc.publicnode.com |
| polygon | 137 | POL | https://polygon-bor-rpc.publicnode.com |
| bsc | 56 | BNB | https://bsc-rpc.publicnode.com |
| avalanche | 43114 | AVAX | https://avalanche-c-chain-rpc.publicnode.com |
| tron | 728126428 | TRX | https://tron-rpc.publicnode.com |
| sepolia | 11155111 | ETH | https://ethereum-sepolia-rpc.publicnode.com |

RPC precedence: `--rpc <url>`, then `ZWALLET_RPC_<KEY>` (e.g. `ZWALLET_RPC_BASE`,
`ZWALLET_RPC_TRON`), then the default. zwallet checks the endpoint's `eth_chainId` before using
it; on `tron` (Tron's HTTP API, `POST /wallet/*`, no API key) it checks that block 0 is mainnet's
genesis block, whose id ends in the chain id shown (`0x2b6653dc`). TronGrid
(`https://api.trongrid.io`) works as `--rpc` too, but without a key it allowed 3 requests a
second on 2026-10-28, too few for `balance` without `--chain`. An RPC URL may carry an API
key, so zwallet prints only its scheme and host (`https://host/<redacted>`), parsing the URL the
way the HTTP client does. Some providers put the key in a subdomain, so a host label that looks
like a key (a hyphen-free run of 20+ characters, or 12+ characters mixing letters and digits) is
`<redacted>` too wherever the URL or its origin is printed (`chains`, HTTP errors, a chainId
mismatch); the default hosts above stay readable. A non-2xx HTTP answer is reported as
`HTTP <status> from <scheme://host:port>`, never with its body, and an answer that is not the
JSON-RPC expected only by the parse error's kind and position, never the value it quotes. In other
error text the endpoint sends back, zwallet replaces with `<redacted>` the URL's userinfo, query
values, whole path and key-like host labels at any length, and its path segments (and fragment)
of 8+ characters, each as written and percent-decoded; a part shorter than 8 characters only where
it stands as a whole word. A JSON-RPC error's `data` is printed only when it is a string (revert
data).


## Fixed-nonce recovery in the TUI

On a pending EVM send status or transaction detail, `s` reviews a speed-up and `x`
reviews a cancel. Run `V` in this TUI session first. A restarted session enables
these controls only after the signed original's hash and recovered owner are
verified and its nonce, recipient, amount and asset match the local journal.
The journal by itself cannot authorize a replacement. The review fixes the
original hash and nonce; `b` changes the fee bump (default 1.25, range 1.1–3).
Type the payee's nickname, the allowed contract's nickname for an invoke, or
`cancel` for a zero-value native transfer to the verified own address.

Confirmation rechecks the current policy, nonce, funds and total fee. A higher
fee produces another review without requesting a signature. A cancel races the
original: if the original is mined first, the original payment stands.

After an uncertain broadcast, `R` is available only on that send's status while
this session retains its immutable review. It retries the same intent at the
same nonce, even if the journal has changed. After restarting, look the hash up
with `/` or `zwallet tx`. An unknown hash does not prove failure: a fresh send at
a new nonce may pay twice even without `d`, since the repeat check uses only a
10-minute window in a journal editable by any process of this user.
