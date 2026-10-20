# zwallet — agent notes

Spec: `.prd/01-spec.md` and `.prd/02-architecture.md`. Usage and custody model: `README.md`.

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
- **No password file, no password env var, no OS keychain, no network login.**
- Chain-specific code lives under `src/chains/<family>/`; `src/core` stays chain-agnostic.
