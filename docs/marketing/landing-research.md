# Landing-page direction — 2026-11-02

For contributors editing the public introduction. This is a design rationale, not
a security audit or a claim that a particular design will produce a star count.

## Reader and decision

The first visitor uses a coding agent, may be building their first blockchain app,
and wants to understand why moving money still interrupts the work. The page should
answer: what can I delegate, what stays under my control, what works today, and how
do I try it? Commands belong at the installation step, after that understanding.

The accepted monochrome film remains the narrative source. The site adds a quiet
metallic wallet in the hero, then gives the existing explanation a native SVG player.
No star request, invented customer logos, usage statistics or promised returns.

## Primary-source research

Sources reviewed on 2026-11-02. These observations informed the structure; no source
code, paid component or brand asset from the reference sites was copied.

| Source | Observation | Decision for zwallet |
|---|---|---|
| [Linear](https://linear.app/) | The opening states a use case, then uses concrete product scenes to explain it. | Put the task and outcome before implementation details. Follow with one understandable demonstration. |
| [Resend](https://resend.com/) | A restrained dark surface and focused product object support a clear installation/documentation path. | Preserve monochrome, space and one reflective wallet object; make the next step obvious. |
| [affectUI Agent Graph](https://www.affectui.com/components/agent-graph) | Request and response travel along persistent relationships. Its documentation also calls for mobile reflow and textual equivalents. | Reuse our original continuous graph; offer chapter controls, readable descriptions and a reflowed mobile diagram. Do not copy the Pro component. |
| [Coinbase Agentic Wallet overview](https://docs.cdp.coinbase.com/agentic-wallet/welcome) | Existing agent wallets already describe payments and trading, including CLI and MCP options. | Explain zwallet's local setup and signing boundary. Do not claim agent wallets are a new or exclusive category. Mark our future integrations WIP. |
| [GitHub custom Pages workflows](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages) and [configure-pages inputs](https://github.com/actions/configure-pages/blob/main/action.yml) | Custom Actions publishing needs initial Pages configuration. Automatic enablement requires a token beyond the ordinary workflow token. | Use a main-branch deployment with narrowly scoped Pages permissions and a documented administrator setup step; do not add a personal access token. |

## Page progression

1. A wallet for the agent, control for the person; local ownership and setup path.
2. The 36-second explanation: funding, receipts, per-payment refusal, lock, future WIP.
3. The founder's practical interruption: gas, USDT, execution and verification coming
   back to the human.
4. Available source capabilities, supported networks and links to exact evidence.
5. Where rules are enforced, plus limits that affect a funding decision.
6. Services, shopping and investment as visibly labeled WIP integrations.
7. Installation, a first testnet task, and short answers to predictable questions.

The security copy is grounded in [current feature status](../features.md),
[custody and limits](../security.md) and the public CLI paths at main `7be8d51`.
An illustrative balance is not a cumulative budget; an agent-generated report is
not a built-in report screen. Same-user access and repeated allowed payments remain
material limitations. Future investment execution is not an investment strategy or
a promise of profit.

## Motion and delivery

The original SVG timeline lives in [animation.html](../animation.html); the MP4
renderer and the website use that source. Playback is independent of rendering.
The site runs the diagram directly in the browser, pauses it when it leaves view,
and honors reduced-motion preferences. Chapter descriptions provide the same story
as text. The MP4 is an optional separate page, so it is not downloaded on landing.

English is the default; `?lang=ko` switches to Korean. Both versions share the same
structure and feature boundaries. CSS/JavaScript are local, with system fonts and
no analytics, account form, wallet connection or live transaction on this site.

See [site development and publishing](../site.md) for reproduction and deployment.
