# v0.2 — TUI (and the v0.1 signed whitelist it builds on) — SSOT

Status: **planned**, 2026-10-20. Fixed target; do not reinterpret it during the loop. Specs:
- [`../03-tui.md`](../03-tui.md) — the TUI, v0.2.
- [`../04-whitelist.md`](../04-whitelist.md) — the signed whitelist, v0.1 WU1b.

## User statement (verbatim, 2026-10-20)

> 작업 완료후 간단한 tui도 추가해줘 먼저 PRD 만들고 현재 작업 다 완료 된 후에 작업

> zwallet tui
> 현재 선택된 wallet을 포함하여 지갑 리스트업
> 엔터를 누르면 암호 입력해서 1h  정도로 웜업
> 디폴트로 밸런스와 최근 10건 트랜잭션 리스트를 출력해줌
> 기존 tx를 선택하면 해당 내역을 제세히 출력해줌
> 밸런스를 선택하면 네이티브 코인과 usdt usdc등이 있으면 그것들을 출력홰주고 어셋을 선택하면 transfer를 할수 있도록 해줌
> 먼저 whitelist를 추가할수 있도록 함 nickname -> 주소로 그리고 이 화이트리스트는 해당 월릿의 암호로 저장하고 시그니처를 박음. 그래서 이렇게 박은 시그니처가 아니면 전송 자체를 못하게 함 (이건 월릿 시드랑 별도키로 시그니처 박아서 인증할수 있는게 좋을 것 같음)
> 트랜스퍼하고 tx번호로 확인할수 있도록 이를 간단한 tui로

## Fixed scope

**Order:**
- The PRD is written now.
- The signed whitelist (daemon enforcement and CLI) is part of the current v0.1 work, as WU1b. It
  lands before the v0.1.0 tag and before the wallet is funded.
- The TUI starts after v0.1 is complete.

The reason for that order: the v0.1 SSOT already fixed a daemon-enforced signing policy as v0.1
scope, and this whitelist is that policy. The user's "현재 작업 다 완료 된 후에" applies to the
TUI.

**What the TUI does:**
- **"지갑" = the master's named keys** (`zwallet list`), with the currently selected one included
  and marked.
- **Enter → password → warm-up** of about 1h, through the staged unlock. Enter opens the key in
  the TUI. Changing the agent's active key is a separate, explicit action.
- **Home:** balance plus the last 10 transactions. **Tx detail** on select.
- **Balance → assets:** native coin, USDT, USDC and other registry tokens held. **Asset →
  transfer.**
- **After a transfer:** check the result by tx hash (TUI status and `zwallet tx`).

**Whitelist:**
- Entries are nickname → address.
- The secret key lives under the wallet's password. The list is signed with a key **separate from
  the seed**.
- Only a list carrying that signature allows a send.
- It is enforced in the daemon, so it applies to the CLI and the agent as well as the TUI.

**Interpretation, stated:** "해당 월릿의 암호로 저장" is read as follows. The **signing key** is
stored under the wallet password; the list itself is plaintext and signed, so it can be read while
locked. Any same-user process can therefore read who the payees are. They cannot change who the
payees are.

## Decisions taken with this PRD (reversible; recorded here)

From the 2026-10-20 spec reviews (gpt6, zhuge):
- **The whitelist moves into v0.1**, ahead of funding.
- **Approvals are revoke-only.** `transferFrom` is denied. Contract entries need explicit
  selectors and explicit chains.
- **A per-chain fee cap lives in the signed list.** `sign-message` is gated by a list flag.
- **The daemon never holds the whitelist secret.** It only holds the public key and the generation.
- **Edits stop every daemon first,** then leave the wallet locked; `--unlock <dur>` starts a new
  session for a duration typed with the change. (The re-warm to the previous expiry decided here
  was dropped in the WU1b review: it trusted an expiry reported by whatever answered for the
  home.)
- **An invalid list is never re-signed.**
- **Joint rollback of all files is a documented limit, not a claimed protection.**
- **History = the send journal plus a keyless explorer where reachable.** ERC-20 log scanning was
  dropped: the window is short and it surfaces address poisoning.
- **Deleted:** `whitelist init`, `walletId`, the user-added token kind, the history URL override,
  the settings file, the second confirm method.
