# zwallet — agent notes

Spec: `.prd/01-spec.md` and `.prd/02-architecture.md`. Start with `README.md`; command details: `docs/commands.md`; custody model: `docs/security.md`.
Implementation status and planned work: `docs/features.md`.

## Gate (run before every commit; all must pass)

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Live tests (public mainnet RPC, nothing broadcast): `cargo test -- --ignored`.

## Rules

- **Never print secrets.** The mnemonic, the password and private keys must not appear in stdout,
  stderr, logs, error messages, `Debug` output, or test output. Error text from BIP39/crypto
  libraries can quote secret input; wrap it in fixed strings. The only exception is
  `zwallet backup` on an interactive terminal.
- **Tests use a temp `ZWALLET_HOME`** (`/tmp/zwt-*`, short because of the `sun_path` limit) with
  `env_clear()`. Never read or write the real `~/.zwallet`. Tests that start a daemon must lock or
  kill it on exit, including on failure (see `tests/common/mod.rs`).
- **The daemon holds only the key**: derive and sign, nothing else. No RPC, no file writes, no
  extra features in `src/daemon`. Everything else belongs in the one-shot CLI.
- **The daemon never trusts the CLI's intent.** `sign_tx` carries only the unsigned signing payload
  bytes; the daemon decodes them in `chains::evm::tx::decode_unsigned`, and the signing policy
  (`chains::evm::policy::check_tx`, against the verified whitelist) decides that decoded view
  before `sign_decoded`. Do not add amount/token/recipient fields to the request.
- **Policy state lives in the sealed payload** (`SealedPayload.policy` in `core/store.rs`, under
  the password-derived key), never in plaintext metadata.
- **No password file, no password env var, no OS keychain, no network login.**
- Chain-specific code lives under `src/chains/<family>/`; `src/core` stays chain-agnostic.

## Documentation

Write for the reader: keep the README focused on evaluation, installation and first use.
Mark planned or incomplete behavior with a WIP badge, including design mockups; check the
public CLI/TUI path before describing a feature as available. Keep `docs/features.md` in sync.

Every commit must include `Co-authored-by: z <z@2lab.ai>`.
