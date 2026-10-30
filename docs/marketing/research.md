# Positioning research — 2026-10-29

[README](../../README.md) · [Launch kit](launch-kit.md) · [Future PRD](../../.prd/07-agent-economy.md)

Audience: maintainers preparing an open-source launch. Findings below come from
primary documentation/repositories accessed on 2026-10-29, not installation tests or
independent security audits. Product behavior and requirements can change; recheck
before publishing a competitive claim. This is a focused comparison, not an exhaustive
market census.

## Decision

Lead with **“Give your agent a wallet. Keep control.”** The first reader should
recognize a practical interruption: AI can build an app, but still hands moving money
and checking results back to the human. General AI users and people building with
coding agents should understand the benefit without reading commands.

Use a short illustrated network story: task funds → agent action → wallet rules →
result and receipts. Show an allowed request passing, an over-limit request
being refused and access being closed. Show investment/payment/shopping possibilities
with WIP labels. Keep the runnable CLI recording as optional evidence for readers
who want to inspect the implementation.

The defensible distinction is the combination of a local Rust CLI, human-controlled
unlock, a small key daemon enforcing signed policy and user-selected RPC. These are
source-backed properties, not evidence of being unique, safest or audited.

## Closest alternatives

| Project / source | What its own documentation says | Implication for zwallet |
|---|---|---|
| [Trust Wallet Agent Kit authentication](https://developer.trustwallet.com/developer/agent-sdk/authentication) and [CLI](https://developer.trustwallet.com/developer/agent-sdk/cli-reference) | Documented API use requires an access ID and HMAC secret obtained through its portal; CLI initialization stores credentials. | “No wallet-service signup or API credentials for zwallet” is a useful operational distinction. Do not turn this into an unsupported claim about every TWAK code path or its custody. |
| [Tether WDK CLI](https://github.com/tetherto/wdk-cli) and [wallet guide](https://docs.wdk.tether.io/cli/guides/manage-wallets/) | Apache-2.0 multi-chain CLI/MCP with local signing, encrypted seeds and configurable per-wallet TTL; its README describes five minutes as the default and `--ttl 0` as unlimited. | A close open-source alternative exists. Do not claim WDK always forces re-unlock every five minutes or requires a hosted account for all use. Compare architecture and current policy needs, not a stale TTL observation. |
| [Coinbase Agentic Wallet CLI](https://docs.cdp.coinbase.com/agentic-wallet/cli/welcome) and [quickstart](https://docs.cdp.coinbase.com/agentic-wallet/cli/quickstart) | Built on CDP, with email-OTP authentication, stablecoin/trading/payment workflows and documented spending controls. | Local no-login operation is a distinction; investment/payment breadth is not a current zwallet advantage. This row is about the Agentic Wallet CLI, not every Coinbase open-source wallet provider. |
| [Coinbase AgentKit](https://github.com/coinbase/agentkit) | Open-source agent tooling and wallet/action integrations. | Treat it as a potential surrounding agent ecosystem, not proof that all agent integrations need a hosted wallet. Review the specific wallet provider when making setup claims. |
| [SmartAgentKit](https://github.com/smartagentkit/smartagentkit) | MIT SDK/contracts using Safe/ERC-7579 smart accounts, on-chain policy hooks and ERC-4337 bundling. | On-chain enforcement is a different boundary. zwallet's local EOA model does not provide equivalent protection against a compromised host. Do not claim it does. |

No star counts or popularity ranking: neither establishes product fit or security.
No competitors were installed or subjected to adversarial testing for this comparison.

## Adjacent opportunity

[x402](https://docs.x402.org/introduction) defines programmatic HTTP payments for
services. It is a protocol to evaluate for future adapters, not a feature zwallet
already implements. Its [payment identifiers](https://docs.x402.org/extensions/payment-identifier)
and receipt/settlement concepts merit investigation when designing idempotent paid
requests. Validate the chosen scheme's signing and policy requirements before building.

Investment and purchase automation broaden the vision, but the immediately verifiable
story remains development allocation → permitted execution → results. Lead with a
plain-language explanation and let the WIP segment explain the ambition.

## Claims to use / avoid

| Use | Avoid |
|---|---|
| Local keys; no zwallet service account; direct chosen RPC | The only open-source agent wallet; zero dependencies; no third parties ever |
| Signed payee/contract rules and optional per-transaction limits | A guaranteed task budget, agent sandbox or protection from every compromised-host attack |
| Current CLI JSON/receipts support agent-generated reports | Built-in automatic reports or an immutable audit ledger |
| Future agents could execute investment and purchase mandates | Autonomous profits, guaranteed returns, supported shopping today |
| Real local EVM recording with valueless ETH | Mainnet proof, real USDT payment, real merchant delivery or live LLM session |

## Launch fit

The public story targets general AI users and people building apps with coding
agents. They should recognize the problem, understand what the wallet provides and
want to try or share it. Developers and security reviewers get commands, execution
evidence and source links one level deeper.

The owner's reach goal is **10,000 GitHub stars**, supported by widespread use and
sharing. It is a goal, not a forecast. A clear story, a working first-run path and
useful examples should earn that attention; star counts alone do not validate the
product.

[Show HN guidance](https://news.ycombinator.com/showhn.html) asks for something readers
can try, not just a landing page or announcement. Pair the visual story with the
quick start and optional local reproduction. Post a founder explanation and answer
questions about custody, policy, retries and competitors; do not solicit votes or
manufacture endorsements.
For other communities, check their current self-promotion rules before posting.
