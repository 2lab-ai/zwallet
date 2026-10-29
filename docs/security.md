# Security and custody

[Back to README](../README.md) · [Command reference](commands.md) · [Feature status](features.md)

Read this before funding a wallet or giving an agent access to an unlocked session.
The whitelist controls what the daemon signs; it does not isolate the agent from other
processes running as the same OS user.

## Custody model

- **Home**: `$ZWALLET_HOME` or `~/.zwallet`, mode 0700.
- **`wallet.json`** (0600, written atomically): a sealed section encrypted with AES-256-GCM under a
  scrypt key (N=2^18, r=8, p=1, 256 MiB; parameters, a 32-byte salt and nonce stored in the file;
  only r=8, p=1, N from 2^15 to 2^18 and a salt of exactly 32 bytes are accepted, checked before
  any key derivation), plus non-secret
  metadata: `createdAt`, `active`, `nextIndex`, and
  `keys: [{ name, index, createdAt, addresses: { <chain>: <address> } }]`. The sealed plaintext is
  a versioned object `{ version, mnemonic, policy }`: one BIP39 English mnemonic (no BIP39
  passphrase) and a `policy` object holding the whitelist's anchor (`whitelistPub`,
  `whitelistGen`, `whitelistDigest`) and the salt its key is derived from (`whitelistSalt`, 32
  random bytes drawn by the first `whitelist add` and by every `reset`); empty until the first
  `whitelist add`, and carried through every other write intact. The policy lives under the
  password because anything outside it can be edited by the same OS user without the password.
  The daemon keeps the anchor, never the salt.
- **`whitelist.json`** (0600, next to `wallet.json`): the signed list in plain JSON. Its ed25519
  signing key is in no file: each change derives it from the password and `whitelistSalt` (scrypt
  over `zwallet/whitelist-key/v1\n` followed by the salt, with the parameters that opened
  `wallet.json`), separately from the AES key that opens `wallet.json`. Only `whitelist` changes
  write the list, under `w.lock`, which orders zwallet's own writers but cannot stop a same-user
  process that ignores it: what exposes such a write is the signature and the seal.
  `whitelist.next.json` exists while a change is being published, or after one was cut off (see
  **Known limits**). A `whitelist.key` left by zwallet 0.1.0 is no longer read and is not removed;
  you can delete it after `whitelist reset`.
- **Password**: held by the human only, at least 12 characters. There is no password file and no
  password env variable; it is typed at a terminal prompt (no echo) or piped with
  `--password-stdin`, and read with raw reads into one wiped buffer. Nothing on disk decrypts the
  wallet by itself. `unlock` hands it to its own daemon child over a socketpair, never over the
  socket file (see **Daemon**). `create` writes a
  temp file, reads it back from disk, decrypts it and checks the addresses and the phrase, and only
  then links it into place as `wallet.json`; a failure before that removes only its temp file (a
  home it created stays, with `w.lock`). Once `wallet.json` is in place it is never removed: a
  later failure says recovery is needed and names the file. Every write of `wallet.json` holds an
  exclusive lock on `w.lock` (an empty file, never deleted). `wallet.json` is only read without
  following a symlink and without blocking, and only as a regular file of this user's of at most
  1 MiB, so a FIFO or link planted in its place fails fast instead of hanging a reader.
- **Keys**: named subkeys of the master by index, in creation order, never reused. EVM uses
  `m/44'/60'/0'/0/<index>` (the standard BIP44 Ethereum account order), so the phrase imports into standard
  wallets with the same accounts. One address per key on every EVM chain. Indices stay below
  2^31, where BIP32's hardened range starts (`VALIDATION_ERROR` at or above it).
- **Daemon**: `unlock` pins the zwallet binary before it asks for the password. On Linux that is
  the running image (`/proc/self/exe`), so a file swapped in at the binary's path while the human
  types is never started. On macOS it is the file's identity, checked again right before the
  start: a binary swapped in while the human types is refused. One swapped in between that check
  and the exec is still started (macOS has no `fexecve`). Once the started process asks for
  input, and before the password is written, `unlock` refuses it unless it runs this build's code
  (its `csops` cdhash equals `unlock`'s own and it is `CS_VALID`): it is stopped, given nothing,
  and `unlock` fails with `VALIDATION_ERROR`. That is partial hardening: it does not prove that
  no other process holds the daemon's channel, so a binary swapped in as it starts can still pass
  the password to a process it forked before it exec'd this build. A build that is not
  code-signed cannot make the check and refuses to unlock before the prompt (see [Install](../README.md#install)).
  One swapped in before `unlock` ran gets the password on any OS. Against a swapped binary the
  only real protection is to install zwallet where an agent cannot write it (see **Known
  limits**). On Linux, `unlock` and `create --unlock` also
  refuse, before the password prompt, when `kernel.yama.ptrace_scope` is 0 or Yama is missing
  (any same-user process could attach to the new daemon during its start-up); set it to 1 or
  more, or pass `--allow-ptrace-scope-0` to accept that. `unlock` then starts the pinned binary
  as its own child `zwallet daemon --home <canonical home>` (own session, empty environment, no
  inherited descriptor but its channel: the open descriptors are listed and marked
  close-on-exec, not just those below the soft descriptor limit; stderr is `/dev/null`). The
  channel is one end of a socketpair on the child's fds 0 and 1, whose other end only `unlock`
  holds: unlike a pipe, a socket cannot be reopened through `/proc/<pid>/fd`, so no same-user
  process can read the password from it or write a fake readiness line into it, even before the
  child turns non-dumpable (on Linux the daemon refuses to start on anything but a socket). Over
  that channel:
  1. the child, whose first step (like every zwallet process's) disabled core dumps and, on
     Linux, made it non-dumpable, asks for input;
  2. `unlock` writes the TTL and password into the channel. The password never goes over the
     socket file, argv or env;
  3. the child decrypts the wallet and says so. A wrong password ends here, and a session that was
     already running is left untouched;
  4. only now does `unlock` stop the daemons already serving this home (see **`lock`**), then
     tells the child to go on;
  5. the child takes an exclusive lock on `d.lock` for its whole life (waiting up to 5 s),
     replaces whatever is at `$ZWALLET_HOME/d.sock`, binds it (0600; peer uid checked on both
     ends), and reports `{ locked, expiresAt, active, index, address, pid }`.

  `unlock` reports success only after that line, and kills the child on any failure or timeout.
  If `unlock` is gone by then (killed after it told the child to go on), the child's readiness
  write fails, and it wipes the seed and exits instead of serving.
  Concurrent unlocks take turns on `u.lock`, held from after the password prompt until the new
  daemon is ready or has failed (another `unlock` waits up to 30 s for it). A listener squatting
  on `d.sock` gets only requests that carry nothing and is replaced; a process holding `d.lock`
  makes `unlock` fail visibly. Neither ever receives the password.
- **`lock`** is the panic button: it ends every zwallet session of this user, not only this
  home's, so a home moved or re-linked since its unlock cannot hide a daemon. It reads no wallet
  home at all (it works with `ZWALLET_HOME` and `HOME` unset, and nothing at a home path, such as
  a stalled mount, can hold it up): it finds every process of this user whose arguments are
  `daemon --home <absolute path>` in the OS process table (macOS `KERN_PROCARGS2`, Linux
  `/proc/<pid>/cmdline`), sends it SIGTERM (with SIGCONT, so a stopped daemon still wipes its
  seed), and SIGKILL after 2 s. It reports `locked: true` with the pids in `stopped` only when
  none is left; otherwise it fails naming the pids. `unlock` stops the old daemons of its own
  home the same way (the home matched by path first, then by inode, with each stat of another
  process's path bounded to 1 s so a hanging mount cannot stall it).
  `status` reads the same table: any daemon process for this home means `locked: false` (with
  `daemonPids`), whatever answers on `d.sock`; `daemonPid` and `expiresAt` come from `d.sock`'s
  reply only when that reply is from one of those processes. `otherDaemonPids` (always present)
  lists this user's daemons for other homes, which `zwallet lock` stops too; human output says so
  when there are any. On macOS a process list that fills its buffer is read again with more
  room, and one that still fills it is an error, never a shorter list. File names (`d.sock`,
  `d.lock`) never decide which daemons exist, since any same-user process can rename or replace
  them.
- **Daemon duties**: the daemon keeps only the master seed and the whitelist's sealed anchor in
  memory, and does nothing but derive addresses and sign; it makes no network request. Before it
  signs a transaction it reads `whitelist.json` and checks it against the anchor (see [the whitelist](commands.md#the-whitelist)). That read runs outside the session's lock, with at most 4 reads in flight, and a
  signing request whose list read cannot complete within the 2 s deadline (waiting for a free slot
  counts against it) is refused (`WHITELIST_INVALID`, "could not be read in time"), so a stalled
  file cannot hold up `lock` or expiry. The session, `wallet.json`'s ciphertext and, last, the
  expiry are checked again before signing, so a read that finishes after `lock`, expiry or a re-seal
  signs nothing. For a transaction it receives only the unsigned signing payload bytes and decodes
  them itself: type, chain id, recipient, value, and the ABI method the calldata calls
  (`transfer-call` / `approve-call` / `transfer-from-call`, each `assetType: "unverified"`, else
  `contract-call`; see [decoded signing actions](commands.md#send-journal-and-retry-safety)). It refuses anything zwallet does not produce
  (other tx types, access lists, contract creation, non-canonical encodings, calldata for those
  three methods that is not their canonical ABI encoding or that also sends native value) and signs
  exactly the bytes it decoded. It never trusts intent fields from the CLI, which is the
  agent-controlled side. On `lock`, at expiry (wall clock), or on SIGTERM it wipes the seed and
  exits. A reboot or a dead daemon means locked. Everything else (RPC, ABI, tx building, broadcast)
  runs in the one-shot CLI process, which checks every signature it gets back by recovering the
  signer.
- **Daemon lifetime**: each connection is served in its own task with a 10 s I/O budget, so a
  silent client never delays expiry. A separate monitor and requests other than `lock`
  (including `status`) check that `wallet.json`'s ciphertext is still the one it decrypted and
  that `d.sock` is still its own socket; otherwise the daemon wipes the seed and exits. The socket check is
  best effort (a same-user process can race it); `lock` does not depend on it. Wallet and socket
  inspection runs off the async event loop and outside the session mutex, on at most four
  secret-free workers with a two-second deadline including slot wait. Expiry and lock do not
  wait for those workers. Inspection failure or timeout erases the session, and late results
  cannot revive it or sign. The password/decrypt startup sequence is outside this guarantee.
  Every zwallet process disables core dumps as its first step, and on Linux also becomes non-dumpable (out of
  reach of same-user `/proc/<pid>/mem`, `/proc/<pid>/fd` and new ptrace attachments) from that
  step on; not before it, since exec makes every process dumpable again.
- **Secrets are never printed**: the mnemonic, password and private keys appear in no output, log
  or error. The single exception is `backup`, which prints the phrase only when stdin and stdout
  are both an interactive terminal, after the password is entered again.

## Known limits (same user)

The threat zwallet is built against is an agent (or anything prompt-injected into one) running as
the same OS user, with read and write access to the wallet home: it must never get the password or
the phrase, `lock` must end all signing, and no output may present its address as verified. Some
things a same-user process can still do, and what to do about them:

- **Swap the zwallet binary.** A binary replaced before `unlock` (or `create`, `backup`,
  `--verify`) runs gets the password. On macOS one swapped in between `unlock`'s identity check
  and its exec of the daemon (there is no `fexecve`) is started; `unlock` refuses it, and gives
  it nothing, when it does not run this build's code once it asks for input (same cdhash,
  `CS_VALID`). That check does not prove that no other process holds the daemon's channel: the
  swapped-in program can fork a process that keeps the channel, then exec this build, which
  passes, and that process can read the password. (It could also exec this build with
  `DYLD_INSERT_LIBRARIES`; the binaries zwallet ships carry the hardened runtime, under which dyld
  ignores that variable while System Integrity Protection is on, but a build from source does
  not.) A Homebrew-installed binary is
  writable by the user, and so by the agent. For
  real protection install zwallet where the agent cannot write (e.g. a root-owned directory), or
  run the agent as a different OS user.
- **Read the human's terminal** while the password is typed: a same-user process can open the
  terminal device the prompt reads. The same access lets it write there too, so it can draw
  whatever it likes on that terminal: the escaping above covers zwallet's own output only.
- **Forge a line within a budget.** Text from outside is bounded and cannot break a line or draw
  a glyph, but padded to the terminal's width it can start a visual row at column 0 by soft wrap,
  and on a 21-column terminal a 512-character budget fits a version-1 QR code of a short link
  (not an address, which needs 625 modules). A same-user process can draw anything on the
  terminal directly anyway; zwallet's own output never presents such text as verified.
- **Read a piped password** (`--password-stdin` from a pipe, Linux): the process writing into the
  pipe, and zwallet itself until its first step makes it non-dumpable, are dumpable, so a
  same-user process can open the pipe through `/proc/<pid>/fd` and race zwallet to read the
  password. This is the same class as reading the terminal; prefer the terminal prompt.
- **Log keystrokes under X11**, where any client can read another's input.
- **Attach a debugger on macOS** to a build from source: a same-user debugger (where the system
  allows one) can read a zwallet process's memory. The binaries zwallet ships carry the hardened
  runtime without the `get-task-allow` entitlement, so a same-user debugger cannot attach to them
  unless System Integrity Protection is off.
- **Linux with `kernel.yama.ptrace_scope` 0** (or without Yama): any same-user process can
  attach to a new daemon before it turns non-dumpable and stay attached (or copy its descriptors
  with `pidfd_getfd`, which takes the same permission). `unlock` refuses this by
  default (`--allow-ptrace-scope-0` accepts it). With `ptrace_scope` 1, a process that is an
  ancestor of `zwallet unlock` (e.g. the shell an agent started, if the human types `unlock`
  into it) still can: run `unlock` from a terminal the agent did not start.
- **Linux hosts with `/proc` mounted `hidepid`** (`1`/`noaccess`, `2`/`invisible`,
  `4`/`ptraceable`, with or without `gid=`) are refused: there the kernel hides a non-dumpable
  process such as the key daemon even from its own user, so `lock` could not find a daemon to
  stop. `unlock` and `create --unlock` refuse to start one before the password prompt (there is
  no flag to accept this), and `lock` and `status` fail instead of reporting the wallet locked.
  The same holds when `/proc/self/mountinfo` cannot be read. `subset=pid` is fine. Elsewhere,
  a `/proc/<pid>` file that cannot be read is skipped only when the process is gone or this user
  may not signal it (`kill(pid, 0)` answers EPERM: another user's process, or one a security
  module shields). One whose `status` shows this user's effective uid, or that this user may
  signal (`kill(pid, 0)` succeeds: also for a process whose real or saved uid is this user's,
  and for any process under root or CAP_KILL), makes `lock` and `status` fail ("refusing to
  trust a partial daemon list"). Signal permission is what is checked, not ownership.
- **Root** defeats all of this.

The whitelist ([`.prd/04-whitelist.md`](../.prd/04-whitelist.md)) limits what an unlocked daemon
signs for such an agent. What it does not do:

- **Optional per-transaction limits.** Without configured signed limits, an agent can send the
  whole balance in one transfer to a listed payee. [Native/token/calldata limits](commands.md#per-transaction-limits)
  constrain individual decoded transactions, not cumulative spending or authority granted by
  message signatures. Repeated transfers can still drain the balance. Cumulative budgets remain
  ![WIP](https://img.shields.io/badge/status-WIP-yellow), tracked in [#26](https://github.com/2lab-ai/zwallet/issues/26).
- **Listed contracts.** A contract entry lets the agent call its listed selectors with any
  arguments, within configured native-value and calldata limits. The contract may forward
  value or move tokens indirectly; direct token-transfer caps do not constrain those effects.
  List only contracts and selectors whose effects you accept from any caller.
- **Joint rollback.** The seal is authenticated but not fresh: an older `wallet.json` still opens
  with the password. Restoring an older `wallet.json` and `whitelist.json` together brings back the
  older list, revoked entries included; nothing stored in the home can tell. (The older
  `wallet.json` seals its own salt, so the same password derives the key it names.) `unlock`,
  `status` and `whitelist list` show the generation and the entries: check them. Restoring
  `wallet.json` alone to an older edit fails closed: its anchor does not match the list, and every
  signature is refused. The reason names the cause by the list it finds above the seal (gen N, the
  seal at gen M): two or more edits above, under the sealed key, "whitelist.json (gen N) is newer
  than the seal (gen M): wallet.json was rolled back"; one edit above, an edit that never finished
  sealing or a rolled-back `wallet.json`; another key, a rollback across a reset, a reset that never
  finished sealing, or a foreign file. If wallet.json is an older copy, restore the current one if
  you have it; zwallet keeps no copy of `wallet.json` (`zwallet backup` prints only the recovery
  phrase), so keep your own copy after each whitelist change. Otherwise `whitelist reset`, and set
  the list up again. Strong joint-rollback protection remains a design issue
  [#27](https://github.com/2lab-ai/zwallet/issues/27); no external freshness store is added.
- **zwallet never suggests restoring an older whitelist.json.** A copy restored by hand that
  verifies against the sealed digest is the older list, so restoring one can complete a
  rollback (W5b). zwallet keeps no counter outside the home, so it cannot tell such a copy from
  the current list. If the list is damaged, deleted or edited by accident, the way back is the
  current `wallet.json` (if an older one went back) or `whitelist reset`, adding the entries
  again by hand; the reset summary states the cost before the password.
- **Rollback to before the first edit.** A `wallet.json` from before the first `whitelist add`
  seals no whitelist, so the daemon sees "no whitelist yet": it signs no payee send and no
  contract call, but it does sign native self-transfers under the **built-in default caps**,
  which may be looser than the caps you signed. `unlock` and `status` show it (`whitelist:
  { gen: null, verified: false }`), and every whitelist edit refuses while a list file exists;
  put the current `wallet.json` back (your own copy from after the last change), or `whitelist
  reset` (every fee cap back to the default, no optional transaction limits, no payee and no contract).
- **Same password.** The whitelist key is derived from the wallet password, so the whitelist
  adds no second factor: whoever has the password can change the list. What it buys is that the
  daemon never holds the key that signs the list. Any change to the password or to
  `wallet.json`'s KDF parameters (zwallet has no command for either yet) must sign the list again
  under the key they then derive; otherwise every change refuses the key.
- **The staged list.** A change cut off between its re-seal and writing `whitelist.json` leaves
  the new list in `whitelist.next.json`. The daemon and `whitelist list --verify` use it, since
  it is exactly the list the seal names (generation and digest), and the next change moves it
  into place.
- **A change ends every session.** Like `lock`, a whitelist change stops every zwallet daemon of
  this user, for every home, and leaves them locked; `--unlock <duration>` starts a new session
  for this home only.
- **The fee cap is per transaction.** It bounds each signature, not a total: repeated
  transactions can each spend up to the cap.
- **The L1 data fee is outside the cap** on the OP-stack chains (Base, Optimism): the cap covers
  `gasLimit × maxFeePerGas`, and the L1 fee for calldata is charged on top. Calldata can be up
  to about 512 KB unless a lower signed byte limit is configured. A calldata limit bounds
  payload size; it does not cap the L1 fee or total transaction cost.

The deferred cumulative-budget contract is recorded in [the budget design](../.prd/08-cumulative-budget-design.md).
The boundary and future acceptance gates for joint policy rollback are in [the freshness design](../.prd/09-policy-freshness-design.md).
These are design decisions, not additional runtime protections.
