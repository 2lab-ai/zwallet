# Development

[Back to README](../README.md) · [Feature status and review evidence](features.md)

## Checks

Gate (must be green before every commit):

```sh
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

Live tests against public mainnet RPC (read-only; nothing is broadcast):

```sh
cargo test -- --ignored
```

Tests always use a temporary `ZWALLET_HOME` under `/tmp/zwt-*` (short, for the socket path limit)
and never touch the real `~/.zwallet`. Layout: `src/core` (custody, seed, amounts, durations,
errors, daemon protocol), `src/chains` (registry and family traits; `src/chains/evm`),
`src/daemon` (key holder), `src/cli.rs` and `src/commands.rs` (CLI), `src/wallet.rs`
(shared wallet reads), and `src/tui` (terminal UI). Design specifications and planning history
live in [`.prd/`](../.prd/); they include features that have not shipped. Use
[feature status](features.md) for the current implementation boundary.

## Release

- **Stable**: set `version` in `Cargo.toml`, merge to `main`, then push the tag `vX.Y.Z` with the
  same version. `release.yml` refuses a tag that does not match `Cargo.toml`, builds the four
  binaries, and publishes them with `SHA256SUMS` as the GitHub release `vX.Y.Z`.
- **Preview**: every push to `main` publishes the prerelease
  `preview-<YYYY-MM-DD-HHMM>-<sha12>` with the same assets (`preview.yml`; the 15 newest are kept).
- The `2lab-ai/homebrew-tap` formulae `zwallet` and `zwallet-preview` pick up new releases on the
  tap's schedule.
