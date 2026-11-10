# v0.4 — TUI look — SSOT

Status: **in progress**, 2026-11-10. Fixed target; do not reinterpret it during the loop. Spec:
[`../10-tui-look.md`](../10-tui-look.md). Driver: [`loop.md`](loop.md).

## Owner's request (verbatim, 2026-11-10)

> 미리 할수 있는 것들부터 작업해 tui 개 좇같으니까 2lab.ai/tui 이용해서 개선해줘

"미리 할수 있는 것들" are the agent's open items while TR23 waits for the owner's funds
([`../v0-3-tron/loop.md`](../v0-3-tron/loop.md)): the rebase warning on PR #59 (posted 2026-11-10)
and the two open TUI test notes ([`../v0-2-tui/loop.md`](../v0-2-tui/loop.md) §Open test notes),
which unit L1 carries. "2lab.ai/tui" is the `twolab-tui` widget library
(<https://2lab.ai/tui>, `github.com/2lab-ai/tui`).

## Standing constraints (binding)

- Every function stays reachable from the TUI, with the keys it has today
  ([`../v0-2-tui/ssot.md`](../v0-2-tui/ssot.md)).
- 03-tui.md §Display integrity holds, with amendments F1 and F2 of
  [`../10-tui-look.md`](../10-tui-look.md) only.
- No mnemonic, password or private key is ever printed; tests never touch the real `~/.zwallet` and
  stop no daemon outside `/tmp/zwt-` (DEV.md).
- The owner's live 1y session stays as it is: merging publishes a preview, and the owner decides
  when to upgrade; upgrading before TR23 locks the wallet (cdhash), so the agent does not ask for it.

## Done means

Units L1–L3 of [`../10-tui-look.md`](../10-tui-look.md) merged, each with its acceptance rows
closed in [`loop.done.md`](loop.done.md), shipped to a preview, and the real-terminal QA at 80x24
and 120x40 green on that preview.
