# Your agent. Your money. Your rules.

[Watch in English](https://ai-insightquest.github.io/zwallet/?v=flow-36) ·
[한국어로 보기](https://ai-insightquest.github.io/zwallet/?lang=ko&v=flow-36) · [Back to zwallet](../../README.md)

[English MP4](../../docs/assets/agent-story.mp4) · [한국어 MP4](../../docs/assets/agent-story-ko.mp4)

A 36-second explanation for people who use AI agents or build apps with them.
One continuous diagram follows the person, their agent, the wallet and a payment test.
Funds, requests and results travel along the same connections, so you can follow
what happens without reading a terminal. All explanations are on screen; sound is
not required.

The example is illustrated, not a live transaction, a product dashboard or a claim
that the agent's task report is a built-in wallet screen. Amounts and network fees
are illustrative. USDT payments and ETH gas remain separate throughout.

## Story and transcript

| Time | What you see | What it explains |
|---|---|---|
| 00:00–00:04 | You assign a payment-testing task and fund the wallet with 50 USDT plus 0.01 ETH for gas. | “Fund the task. Your agent does the work.” |
| 00:04–00:09 | Your agent requests 2 USDT. The wallet checks the allowed recipient, a 5 USDT cap per payment and the access window. The payment reaches the test. | The wallet checks a request before signing. |
| 00:09–00:14.5 | Confirmation returns to the agent, then the agent reports to you. The summary shows 2 USDT spent, 0.00002 ETH in fees, and 48 USDT plus 0.00998 ETH left. | “Get the result. And the receipts.” |
| 00:14.5–00:20 | A 20 USDT request stops at the wallet. It gets no signature and sends no additional payment. | “Over the limit? It stops here.” A per-payment cap is not a cumulative budget. |
| 00:20–00:24.5 | You lock the wallet. A new request cannot proceed. | “You decide when access ends.” Locking stops new signing; it does not reverse transactions already sent. |
| 00:24.5–00:32 | The same wallet connects to services, shopping and investing. All three branches carry WIP labels, with a persistent FUTURE / WIP heading. | “A wallet for what agents do next.” These integrations are not implemented; investing can result in losses. |
| 00:32–00:36 | The graph stays in view with the product idea and repository address. | “Your agent. Your money. Your rules.” Open source, local keys, no zwallet service signup. |

### 한국어

1. **작업비를 맡기면, 일은 에이전트가.** “결제 기능 만들고, 테스트까지 끝내줘.”
   작업비 50 USDT와 가스용 0.01 ETH를 작업용 지갑에 넣습니다.
2. **규칙 안에서 결제합니다.** 에이전트가 2 USDT를 요청하면 지갑이 수취인,
   1회 5 USDT 한도, 남은 접근 시간을 확인합니다.
3. **내가 받는 건, 결과와 사용 내역.** 결제 확인이 에이전트에게 돌아가고,
   에이전트는 결제 금액·네트워크 수수료·남은 작업비를 보고합니다.
4. **정한 선을 넘으면, 여기서 멈춤.** 20 USDT 요청은 1회 한도를 넘으므로
   서명하지 않습니다. 추가 결제 없이 잔액이 유지됩니다.
5. **접근 종료도, 내가 정합니다.** 시간이 만료되거나 직접 잠그면 새 서명이
   종료됩니다. 이미 보낸 거래를 되돌리는 기능은 아닙니다.
6. **그다음은, 에이전트가 할 수 있는 일들.** 서비스 결제·물건 구매·투자 실행으로
   이어질 미래 연동은 모두 WIP입니다. 투자에는 손실이 발생할 수 있습니다.
7. **내 에이전트. 내 돈. 내가 정한 규칙.** 오픈소스 지갑, 내 기기의 키,
   zwallet 서비스 가입 없이 사용하는 구성요소입니다.

## What is available

The wallet currently provides local encrypted keys, permitted EVM transactions and
TRX/registry-USDT transfers on Tron, signed policy, optional per-transaction limits,
timed signing and transaction evidence. An agent can assemble a report from those
outputs; the illustrated summary is not a built-in reporting screen.

Cumulative budgets, native task reports, service-payment, shopping and investment
adapters are ![WIP](https://img.shields.io/badge/status-WIP-yellow).
See the [future PRD](../../.prd/07-agent-economy.md).

Repeated allowed payments can spend the wallet balance. The local signing policy
is not an isolated agent sandbox or a guarantee against a compromised device.
Use a trusted device and read [current capabilities](../../docs/features.md) and
[the custody boundary](../../docs/security.md).

For runnable technical evidence, use the separate
[local CLI example](../agent-budget/README.md).

## Rebuild the media

Requires Node.js 22+, Chrome or Chromium, and FFmpeg. The optional Python package
below supplies FFmpeg if it is not installed separately.

```sh
python3 -m venv /tmp/zwt-story-venv
/tmp/zwt-story-venv/bin/pip install -r examples/agent-story/requirements-media.txt
export PYTHON=/tmp/zwt-story-venv/bin/python
node examples/agent-story/render.mjs
node examples/agent-story/render.mjs --lang ko
```

Set `FFMPEG` to an existing FFmpeg executable to skip the Python dependency.
Set `CHROME` if the browser is outside the standard macOS/Linux locations.
The renderer uses a fresh temporary browser profile and removes it after rendering.
It loads only the local [`motion.html`](motion.html), with no external scripts or assets.

Output is 1600×1000 H.264, 60 fps, 36 seconds (2,160 frames), with matching posters
in `docs/assets/`. Each frame is evaluated at its exact timeline position and painted
before capture. The timeline uses one persistent diagram, eased pulses and fades.
It does not record a browser or terminal at a variable frame rate.

Use `--stills-only` to inspect representative frames before encoding, `--output DIR`
to change the destination, or `--review DIR` for keyframe images. The default review
directory is `zwallet-flow-en` or `zwallet-flow-ko` under the system temporary directory.
Open `motion.html?lang=ko` in a browser to play or scrub the source timeline.
The player includes pause, replay and seek controls.

macOS uses Avenir Next and Apple SD Gothic Neo. Linux uses the system sans-serif font
and Noto Sans CJK KR; install the latter for Korean. Font metrics can vary by platform.

## Playback and visual references

The [playback page](../../docs/index.html) is hosted on the fork's GitHub Pages,
from `docs/` on `docs/agent-story-film`. It provides native video controls, language
links, a transcript and downloads, with no tracking or signup. GitHub's MP4 file view
does not preview these files; link to the playback page. To move hosting, publish
`docs/` on a static host and update the README links.

The [icedac monochrome profile](https://github.com/icedac/icedac/tree/codex/monochrome-profile-2026-10-29)
informs the restrained black, white and gray palette, type and spacing.
[affectUI Agent Graph](https://www.affectui.com/components/agent-graph) informs the
explanation through connected nodes, requests and responses. All geometry, layout,
copy and animation code here are original. The Pro component is not copied or used
as a dependency. There is no star-request end card.
