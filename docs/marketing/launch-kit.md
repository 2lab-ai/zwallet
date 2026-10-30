# Launch kit

Drafts for human review. Nothing in this file has been posted to a community or social
account. [Research](research.md) · [Explainer and transcript](../../examples/agent-story/README.md)

## Message

**Give your agent a wallet. Keep control.**

One sentence: give your AI agent working funds, let it act within your rules, and
get back the results and receipts. zwallet is the open-source wallet behind that workflow.

Suggested GitHub description (manual setting, not applied by this PR):

> A wallet for your AI agent. Give it working funds, set the rules, get receipts. Local keys. Rust / MIT.

Suggested topics (manual settings): `ai-agents`, `wallet`, `cli`, `ethereum`, `evm`, `tron`,
`rust`, `self-custody`, `developer-tools`. Avoid tagging unsupported chains or claiming
implemented investment/checkout integrations.

## Show HN draft

Title: **Show HN: Zwallet — a local wallet CLI for AI agents, with signing rules**

Link: https://github.com/2lab-ai/zwallet

Founder comment:

I built zwallet because my coding agent could write blockchain code, then kept asking
me to send gas or stablecoins, run the payment and check whether it succeeded.
I wanted to fund a dedicated wallet, authorize a bounded set of actions, and get back
the receipts instead of doing each transaction myself.

zwallet is a Rust CLI with local encrypted keys, a human-controlled timed unlock,
signed recipient/contract rules and optional per-transaction limits. An agent uses
JSON commands; the signer checks the transaction bytes. There is no zwallet service
account or mandatory wallet API credential.

The README has a 58-second visual explanation: assign task funds, let the agent work,
watch the wallet block requests outside your rules, and get the result back. It is
an illustration, not a shipped dashboard. An optional reproducible CLI example runs
on a local chain and checks receipts, policy refusal and payment/gas/balance accounting.

This is an early wallet component, not a complete autonomous finance product.
Cumulative budgets, investment adapters and shopping/paid-service workflows are
WIP. A same-user process can reach an unlocked session; a local signing policy is
not a sandbox or the same guarantee as an on-chain smart-account policy.

WDK and other open-source alternatives exist; our research links the closest ones.
I'd like feedback from people giving coding agents on-chain tasks: what do they
keep handing back to you, and what evidence would you want at the end?

## X / short launch thread draft

1. My coding agent could write the blockchain code. Then it asked me to send the gas,
move the tokens, and verify the transaction. I built zwallet to finish that workflow.

2. Fund a dedicated wallet. Set allowed recipients/contracts and per-transaction
limits. Unlock a session. Let your agent execute and return the transaction evidence.
Keys stay on your device. No zwallet service signup. Open source.

3. Watch how it works: your agent requests a payment, the wallet checks your rules,
and you get the result. Future service payments, shopping and investment integrations
are shown as WIP. The reproducible CLI proof is there when you want to go deeper.

4. Try it: https://github.com/2lab-ai/zwallet
What does your agent need to pay for? Current focus: development workflows.
Future direction: accountable task budgets, service payments, investment and purchases.

Attach `docs/assets/agent-story.mp4` (English) or `agent-story-ko.mp4` (Korean).
Keep the WIP labels visible when cropping. The animated graph explains the workflow;
it is not a product dashboard. Keep the first post focused on the human benefit.

Visual tone follows the owner's [monochrome profile](https://github.com/icedac/icedac/tree/codex/monochrome-profile-2026-10-29):
black and white, generous space, medium-weight type and slow fine-line geometry.
Keep that treatment when making crops or follow-up material. Permission/refusal must
stay understandable through labels and symbols, without relying on colored status.

## Korean launch draft

블록체인 개발을 에이전트에게 맡겼는데, 가스 보내달라, 토큰 옮겨달라,
트랜잭션 실행하고 확인해달라는 일을 계속 사람이 해야 했습니다.

그래서 zwallet을 만들었습니다. 작업용 지갑에 필요한 자금을 넣고, 허용할
수취인·컨트랙트와 거래별 한도를 설정하고, 일정 시간 잠금을 풀어줍니다.
에이전트가 허용된 작업을 처리하고 결과와 사용 내역을 보고하게 할 수 있습니다.
키는 로컬에 보관하고, 별도 지갑 서비스 가입 없이 사용할 수 있습니다. Rust / MIT.

README 영상은 사람·에이전트·지갑 사이에서 일이 어떻게 흘러가는지 보여줍니다.
허용된 요청은 통과하고, 허용하지 않은 대상은 막히며, 원하면 접근을 끊습니다.
결과와 사용 내역은 에이전트가 보고합니다. 내장 보고 기능은 아직 WIP입니다.
직접 검증하려는 사람을 위한 로컬 CLI 재현 예제도 별도로 제공합니다.

장기적으로는 이 지갑을 투자·결제·구매 에이전트의 실행 기반으로 확장하려고
합니다. 누적 예산과 해당 연동은 아직 WIP이며, 수익을 보장하는 제품은 아닙니다.

https://github.com/2lab-ai/zwallet
여러분의 에이전트는 어떤 결제나 온체인 작업에서 사람을 다시 부르나요?

## Release sequence and success signals

1. Merge after code/media review and required CI. Check rendered README media from a
   signed-out GitHub session; verify the target release includes the demo's commands.
2. Run the example from a clean checkout on supported macOS/Linux; publish exact
   prerequisites and source revision. Preview builds can precede stable releases.
3. Publish the visual explainer with the founder story; link to the repo and a clear
   first-run path. Reply to substantive questions, correct factual comparisons promptly.
4. Gather workflow issues and reproduction results before expanding the roadmap.

Measure reproducible first runs, time to the first verified local transaction,
quality of integration issues and contributor examples. The reach goal is 10,000
GitHub stars, earned through use and sharing; track stars alongside actual adoption.
Do not invent install metrics or imply video views demonstrate safe real-world use.

## Answers worth preparing

- **Why not WDK?** It is a valid alternative with configurable TTL and wider chain/MCP
  support. zwallet focuses on a local Rust CLI and daemon-enforced signed policy.
- **Can an agent spend everything?** Repeated permitted transactions can consume the
  balance. Cumulative budgets remain WIP; allocation size and policy scope matter.
- **Can it trade or buy things now?** It provides EVM transaction primitives and
  TRX/registry-USDT transfers on Tron under policy. Turnkey strategies, DeFi allowances,
  x402 and checkout adapters are WIP.
- **Does it make profit?** No profit guarantee. Future strategies may lose money.
- **Where are the reports?** CLI JSON, receipts and the send journal are available;
  the demo computes a report externally. Native task reporting is planned.
- **Is it audited?** No independent audit is asserted by this launch. Tests and a
  local demo establish specific behavior, not a blanket security guarantee.
