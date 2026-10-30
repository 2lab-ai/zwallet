# Give your agent a wallet

[Watch in English](https://ai-insightquest.github.io/zwallet/) ·
[한국어로 보기](https://ai-insightquest.github.io/zwallet/?lang=ko) · [Back to zwallet](../../README.md)

[English MP4](../../docs/assets/agent-story.mp4) · [한국어 MP4](../../docs/assets/agent-story-ko.mp4)

A 58-second visual introduction for people who use AI agents or build apps with them.
It explains the problem, the wallet's role, the user's controls and future possibilities.
The moving graph is an illustration of the workflow, not a product dashboard.
The film has large on-screen text and works without sound. Its monochrome layout
uses the owner’s [icedac profile](https://github.com/icedac/icedac/tree/codex/monochrome-profile-2026-10-29)
as the tone reference: dark ground, medium-weight type, generous space, fine gray
rules and slow geometric motion. Allowed/blocked states use words and symbols,
so the distinction does not depend on color.

The playback page is hosted on the project fork's GitHub Pages, using the `docs/`
directory of `docs/agent-story-film`. GitHub's file view does not preview these MP4s;
the page provides native playback, language links, a transcript and downloads.
Its source is [`docs/index.html`](../../docs/index.html), with no external scripts,
tracking or signup. Publishing another copy only requires static hosting of `docs/`;
update the README links to that host when moving it.

## Story and transcript

| Time | What you see | What it means |
|---|---|---|
| 00:00–00:07 | An agent builds an app but keeps sending payment requests back to the person. | “AI can build. You still pay.” |
| 00:07–00:15 | The person puts task funds into a local wallet. The agent then requests payments. | “A working allowance.” You choose funds, permitted destinations and access time. |
| 00:15–00:25 | Requests move through the wallet to the task; the result travels back. The agent produces a summary. | “A task done. A clear account.” Verify the outcome and report the amount, fees and remaining funds. |
| 00:25–00:39 | An allowed destination receives a request. An unapproved destination is blocked at the wallet. The person closes access. | “Your money. Your rules.” Then: “Until you say stop.” Local keys, allowed payees, limits per payment and timed access. Repeated payments can use the allocation; the device must be trusted. |
| 00:39–00:51 | The network expands from development into service payments, shopping and an investment plan. Each future branch is labeled WIP. | “What happens next.” These integrations are future work. Investment outcomes can include losses. |
| 00:51–00:58 | The wallet becomes a single geometric focal point beside the product idea and repository link. | “Give your agent a wallet.” Open source, local keys and no zwallet service signup. |

### 한국어

1. **AI가 만들어도, 결제는 아직 내 몫.** 코드는 알아서 짜면서, 돈 보내고 결제하고
   확인하는 일은 계속 나를 부릅니다.
2. **작업비를 맡기세요.** 작업용 지갑에 자금을 넣고, 어디에, 한 번에 얼마까지,
   언제까지 쓸지 내가 정합니다.
3. **일은 끝내고, 내역은 남기고.** 에이전트가 결제를 처리하고 결과와 사용 내역을 가져옵니다.
4. **내 돈. 내가 정한 규칙.** 허용한 수취인, 거래별 한도,
   접근 시간으로 제어하고 언제든 접근을 종료합니다. 거래별 한도는 누적 예산이
   아니며, 신뢰하는 기기에서 사용해야 합니다.
5. **그다음의 가능성.** 서비스 결제·물건 구매·투자 계획 실행으로
   이어지는 미래 연동은 WIP입니다. 투자에는 손실이 발생할 수 있습니다.
6. **내 에이전트에게 지갑을 주세요.** 내 에이전트. 내 돈. 내가 정한 규칙.

## What is available

The wallet currently provides local encrypted keys, permitted EVM transactions and
TRX/registry-USDT transfers on Tron,
signed policy, optional per-transaction limits, timed signing and transaction evidence.
An agent can assemble a report from those outputs; the illustrated summary is not a
built-in reporting screen. Cumulative budgets, native task reports, service-payment,
shopping and investment adapters are ![WIP](https://img.shields.io/badge/status-WIP-yellow).
See the [future PRD](../../.prd/07-agent-economy.md).

The local signing policy is not an isolated agent sandbox or a guarantee against a
compromised device. See [current capabilities](../../docs/features.md) and
[the custody boundary](../../docs/security.md).

For runnable technical evidence, use the separate
[local CLI example](../agent-budget/README.md). The public explainer uses original
graphics to communicate the idea; it does not present those graphics as an executed
financial transaction, a live agent session or an available GUI.

## Rebuild the media

From the repository root:

```sh
python3 -m venv /tmp/zwt-story-venv
/tmp/zwt-story-venv/bin/pip install -r examples/agent-story/requirements-media.txt
/tmp/zwt-story-venv/bin/python examples/agent-story/render.py
/tmp/zwt-story-venv/bin/python examples/agent-story/render.py --lang ko
```

The renderer writes 1600×900 H.264 MP4s at 60 fps and posters
to `docs/assets/`. Frames are drawn at twice the output resolution and downsampled
for stable fine lines. Link positions are evaluated continuously; state changes and
scene transitions ease between frames. The README uses a still poster linked to the
full-frame-rate video instead of a low-frame-rate GIF. Use `--output` for another directory or `--stills-only` to inspect
the layouts before encoding. Review sheets and individual scene images are written
under `/tmp/zwallet-story-{en,ko}-*`.

macOS uses Avenir Next and Apple SD Gothic Neo from the system. Linux uses DejaVu Sans
and Noto Sans CJK (Korean); install those fonts before rendering. Font metrics can
vary slightly by platform. The encoder is supplied by imageio-ffmpeg.

Motion reference: [affectUI Agent Graph](https://www.affectui.com/components/agent-graph),
specifically requests and responses moving between connected nodes. All geometry,
layouts, copy and animation code here are original; the Pro component is not copied
or included as a dependency.
