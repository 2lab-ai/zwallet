# Launch kit

Drafts for human review. Nothing in this file has been posted to a community or social
account. [Research](research.md) · [Video/reproduction](../../examples/agent-budget/README.md)

## Message

**Fund the task. Let your agent execute. Keep the receipts.**

One sentence: zwallet is an MIT-licensed local wallet CLI that lets a human fund and
authorize an agent's on-chain work, then inspect transaction evidence.

Suggested GitHub description (manual setting, not applied by this PR):

> Local wallet CLI for AI agents. Fund a task, set signing rules, get receipts. Rust / MIT.

Suggested topics (manual settings): `ai-agents`, `wallet`, `cli`, `ethereum`, `evm`,
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

The README has a reproducible local-chain demo: fund 0.020 test ETH, send 0.001,
verify the receipt, reject an over-limit request, and reconcile payment + gas +
remaining funds. The reporting example is a script built on CLI/RPC results, not a
built-in report command. It is scripted, not a live LLM session.

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
Local Rust CLI, JSON output, MIT. No zwallet service signup.

3. The demo runs real CLI commands on a local EVM chain: payment confirmed, over-limit
request refused, spend + gas + balance reconciled. No public funds. Future investment
and shopping scenes are explicitly WIP.

4. Try it: https://github.com/2lab-ai/zwallet
What does your agent need to pay for? Current focus: development workflows.
Future direction: accountable task budgets, service payments, investment and purchases.

Attach `docs/assets/agent-budget.mp4`. Keep the WIP/local-chain labels visible when
cropping. Do not use the future segment alone without its context label.

## Korean launch draft

블록체인 개발을 에이전트에게 맡겼는데, 가스 보내달라, 토큰 옮겨달라,
트랜잭션 실행하고 확인해달라는 일을 계속 사람이 해야 했습니다.

그래서 zwallet을 만들었습니다. 작업용 지갑에 필요한 자금을 넣고, 허용할
수취인·컨트랙트와 거래별 한도를 설정하고, 일정 시간 잠금을 풀어줍니다.
에이전트는 CLI로 작업하고 트랜잭션 근거와 사용 내역을 보고하게 할 수 있습니다.
키는 로컬에 보관하고, 별도 지갑 서비스 가입 없이 사용할 수 있습니다. Rust / MIT.

README 영상에는 실제 CLI 송금, 한도 초과 거절, 영수증 기반 사용액 계산이
들어 있습니다. 로컬 테스트 체인으로 재현할 수 있고 실제 자금은 필요 없습니다.
자동 보고는 현재 예제 스크립트이며, 내장 기능은 아닙니다.

장기적으로는 이 지갑을 투자·결제·구매 에이전트의 실행 기반으로 확장하려고
합니다. 누적 예산과 해당 연동은 아직 WIP이며, 수익을 보장하는 제품은 아닙니다.

https://github.com/2lab-ai/zwallet
여러분의 에이전트는 어떤 결제나 온체인 작업에서 사람을 다시 부르나요?

## Release sequence and success signals

1. Merge after code/media review and required CI. Check rendered README media from a
   signed-out GitHub session; verify the target release includes the demo's commands.
2. Run the example from a clean checkout on supported macOS/Linux; publish exact
   prerequisites and source revision. Preview builds can precede stable releases.
3. Publish the working demo with the founder story; link to the repo rather than a
   signup form. Reply to substantive questions, correct factual comparisons promptly.
4. Gather workflow issues and reproduction results before expanding the roadmap.

Measure reproducible first runs, time to the first verified local transaction,
quality of integration issues and contributor examples. Stars are a secondary signal.
Do not invent install metrics or imply video views demonstrate safe real-world use.

## Answers worth preparing

- **Why not WDK?** It is a valid alternative with configurable TTL and wider chain/MCP
  support. zwallet focuses on a local Rust CLI and daemon-enforced signed policy.
- **Can an agent spend everything?** Repeated permitted transactions can consume the
  balance. Cumulative budgets remain WIP; allocation size and policy scope matter.
- **Can it trade or buy things now?** It provides EVM transaction primitives under
  policy. Turnkey strategies, DeFi allowances, x402 and checkout adapters are WIP.
- **Does it make profit?** No profit guarantee. Future strategies may lose money.
- **Where are the reports?** CLI JSON, receipts and the send journal are available;
  the demo computes a report externally. Native task reporting is planned.
- **Is it audited?** No independent audit is asserted by this launch. Tests and a
  local demo establish specific behavior, not a blanket security guarantee.
