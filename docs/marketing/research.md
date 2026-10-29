# Positioning research — 2026-10-29

[README](../../README.md) · [Launch kit](launch-kit.md) · [Future PRD](../../.prd/07-agent-economy.md)

Audience: maintainers preparing an honest developer launch. Findings below come from
primary documentation/repositories accessed on 2026-10-29, not installation tests or
independent security audits. Product behavior and requirements can change; recheck
before publishing a competitive claim. This is a focused comparison, not an exhaustive
market census.

## Decision

Lead with **“Fund the task. Let your agent execute. Keep the receipts.”** The first
reader should recognize a practical interruption: an agent can write the code but
keeps delegating funding, payments and verification back to the human. Show a complete
permitted execution with a receipt, a rejected request and accounted spend.

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
story remains development allocation → permitted execution → evidence. Lead with the
working demo and let the WIP segment explain the ambition.

## Claims to use / avoid

| Use | Avoid |
|---|---|
| Local keys; no zwallet service account; direct chosen RPC | The only open-source agent wallet; zero dependencies; no third parties ever |
| Signed payee/contract rules and optional per-transaction limits | A guaranteed task budget, agent sandbox or protection from every compromised-host attack |
| Current CLI JSON/receipts support agent-generated reports | Built-in automatic reports or an immutable audit ledger |
| Future agents could execute investment and purchase mandates | Autonomous profits, guaranteed returns, supported shopping today |
| Real local EVM recording with valueless ETH | Mainnet proof, real USDT payment, real merchant delivery or live LLM session |

## Launch fit

Start with shell-capable coding-agent users and EVM developers. They can reproduce
the demo without funding a public wallet and can judge the core friction directly.
Use the wider investment/shopping vision to invite adapter contributors, while every
current-capability statement links to code or a runnable example.

[Show HN guidance](https://news.ycombinator.com/showhn.html) asks for something readers
can try, not just a landing page or announcement. The local demo is the primary call
to action. Post a technical founder explanation and answer questions about custody,
policy, retries and competitors; do not solicit votes or manufacture endorsements.
For other communities, check their current self-promotion rules before posting.
