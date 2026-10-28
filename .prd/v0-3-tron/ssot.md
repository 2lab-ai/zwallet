# v0.3 — Tron — SSOT

Status: **planned**, 2026-10-28. Fixed target; do not reinterpret it during the loop. Spec:
[`../05-tron.md`](../05-tron.md). Units: [`units.md`](units.md). Driver: [`loop.md`](loop.md).

## Owner's request (verbatim, 2026-10-28)

> 같은 시드로 트론 월릿 주소도 생성하고 트론 밸런스 trc20 usdt까지 밸런스 볼수 있고 전송할수 있도록 기능 추가

The same day, for every feature (owned by the TUI task, which rewrites `03-tui.md`):

> tui로 모든 기능 사용할 수 있도록 구현

## Standing owner constraints (verbatim; binding)

- "아니 맥키체인에 저장하지말고 씨발 그냥 너만 사용할수 있으면 된다고" ([`../v0-1/ssot.md`](../v0-1/ssot.md)
  item 3): no OS keychain.
- "트러스트월릿 관련 ㅈ가업한거 다 걷어내" ([`../v0-1/ssot.md`](../v0-1/ssot.md) item 7): nothing
  from Trust Wallet. No `twak`, no `wallet-core`, no Trust Wallet source as authority.
- The password is held by the human only. No mnemonic, password or private key is ever printed
  (`../../CLAUDE.md` §Rules).
- "unlock을 한다음에는 zwallet address 하면 확인할필요 없어야지": after an unlock, reading an address
  asks for no separate verification (the session level, [`../02-architecture.md`](../02-architecture.md)
  §Address trust).

## Fixed scope

**What "the same seed" means.** The key's existing BIP39 seed, no new secret. Key index `i` gets
the Tron address at `m/44'/195'/0'/0/i`. Key 0's address equals TronWeb's default
(`m/44'/195'/0'/0/0`) for the same mnemonic, so a wallet that uses that default path shows it.
Whether another wallet lists later keys in the same order is not claimed
([`../05-tron.md`](../05-tron.md) Q2).

**What ships:**
1. `zwallet address --chain tron` (and the `tron` row of `addresses`), verified by the session
   after an unlock with no prompt, and a QR of a verified `T…` address.
2. `zwallet balance` with Tron in it, and `zwallet balance --chain tron [--token usdt]`: TRX and
   USDT (TRC20, contract `TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t`, 6 decimals).
3. `zwallet transfer --chain tron --to <@nick|T…> --amount <dec> [--token usdt] [--dry-run]
   [--wait]`, TRX and USDT, only to whitelist payees listed for `tron`, with the estimated and the
   maximum cost in TRX shown before sending, and a send refused before signing when the TRX
   balance is below the maximum.
4. `zwallet whitelist add <nick> <T…> --chains tron`, signed as every edit is.
5. `zwallet tx --chain tron <txid>`.
6. For the TUI task: the inputs, outputs, confirmations and errors of each of the above
   ([`../05-tron.md`](../05-tron.md) §TUI).

**What does not change:** `wallet.json`'s format, the sealed anchor, the whitelist file format and
its signature, the daemon protocol (v2). The daemon still holds only the seed, does no RPC, and
decodes and judges every byte it signs.

**What stays out:** staking and energy rental, TRC10, multisig and permission changes, `call` and
`invoke` on Tron, memos, `sign-message` on Tron, USDC on Tron, Tron testnets, API-key headers
([`../05-tron.md`](../05-tron.md) §Non-goals gives each reason).

**Money.** One live mainnet send (TR23) uses the owner's funds. It runs only when the owner has
funded the address and decided to send.
