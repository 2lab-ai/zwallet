# Watch an agent-sized task complete

This is the optional technical appendix. For the public introduction, watch the
[animated explainer](../agent-story/README.md).

The recording below is historical evidence from its stated source revision. Current
source also provides [native task reports](../../docs/task-reports.md); the existing
film, cast and example-computed report are preserved, not represented as a rerun of
that newer command.

[Back to zwallet](../../README.md) · [MP4](../../docs/assets/agent-budget.mp4) ·
[Terminal recording](../../docs/assets/agent-budget.cast) ·
[Execution evidence](../../docs/assets/agent-budget-evidence.json)

This example runs the actual zwallet binary against a disposable local EVM chain.
It gives a wallet 0.020 valueless ETH, pays an allowed recipient 0.001 ETH, verifies
the receipt, refuses a request above the 0.005 ETH per-transaction cap, and accounts
for the payment, gas and remaining funds.

The workflow is scripted; there is no LLM making decisions in the recording. Commands
and selected fields from their real JSON output appear in the terminal recording.
The companion evidence file retains the full command results. The report is computed
by this example from CLI results and the RPC receipt, not by a built-in report command.

The silent video adds readable captions and two imaginative future scenes. Both carry
**CONCEPT / WIP — NOT AVAILABLE** throughout. No investment, merchant order or USDT
payment is executed. Those scenes illustrate the [future PRD](../../.prd/07-agent-economy.md).

## Reproduce the CLI recording

Run from the repository root on macOS or Linux. You need Python 3.10+, the repository's
Rust toolchain and Anvil 1.7.1. The committed recording used macOS arm64, Python 3.14,
Rust 1.95.0 and source revision `99fff4eae7a94fa81374904c2290598b1cf79b36`.
No Python packages are needed for recording.

If Anvil is already installed:

```sh
cargo build --locked
anvil --version
python3 examples/agent-budget/record.py
```

Alternatively, install the pinned Anvil package into a temporary directory using
Node.js/npm, then supply its path:

```sh
demo_tools="$(mktemp -d /tmp/zwt-media-tools-XXXXXX)"
npm install --prefix "$demo_tools" @foundry-rs/anvil@1.7.1
cargo build --locked
python3 examples/agent-budget/record.py --anvil "$demo_tools/node_modules/.bin/anvil"
```

The script writes `agent-budget.cast` and `agent-budget-evidence.json` under
`docs/assets/`. Use `--output /tmp/zwt-demo-output` to keep a trial recording outside
the checkout. Random keys and transaction hashes change on each run; the accounting
and policy assertions must still pass.

The fixture uses a fresh `/tmp/zwt-demo-*` home, a random in-memory password passed
on stdin, a loopback-only Anvil node and valueless ETH assigned by `anvil_setBalance`.
Its chain ID is Sepolia's; **it is not connected to public Sepolia**. The recorder
suppresses Anvil startup secrets and never accesses your normal wallet home.

Only this checkout's debug binary is accepted: its test-only daemon scope prevents
cleanup from locking unrelated real wallets. It also uses the repository tests'
serialization lock, so a concurrent test run may delay recording. On macOS the
script ad-hoc signs that debug binary. Linux must meet zwallet's normal ptrace-policy
requirements; the example does not override them. See [development](../../docs/development.md).
The fixture stops its daemon and Anvil and removes its wallet on normal exit or an
exception. Do not replace its scoped binary with an installed/release binary.

## Render the technical recording

Rendering requires Pillow, pyte and imageio-ffmpeg. Use an isolated Python environment:

```sh
python3 -m venv /tmp/zwt-media-venv
/tmp/zwt-media-venv/bin/pip install -r examples/agent-budget/requirements-media.txt
/tmp/zwt-media-venv/bin/python examples/agent-budget/render.py
```

The renderer needs Menlo/Arial on macOS or DejaVu Sans/DejaVu Sans Mono on Linux.
`imageio-ffmpeg` supplies the encoder. It produces an MP4, looping GIF and poster in
`docs/assets/`, plus a contact sheet at `/tmp/zwallet-launch-contact-sheet.png` for
visual review. `--assets` can point to the recorder's alternate output directory.
The actual terminal output is replayed through pyte; no CLI results are invented.
Only the initial provisioning delay is trimmed. Concept cards are separate graphics.

## Transcript

1. **Fund the task. Let your agent execute.** An agent can write blockchain code but
   still ask the human to send funds and check the transaction. Allocate funds,
   configure signing rules and unlock a session.
2. **Human setup.** The fixture creates a dedicated wallet with 0.020 local ETH,
   allows `@dev-service`, sets a 0.005 ETH native cap and 0.001 ETH execution-fee cap,
   then runs `zwallet unlock 30m --json --password-stdin`. Password input is hidden.
3. **Inspect and preview.** `balance --chain sepolia --json` shows the allocation.
   `transfer --chain sepolia --to @dev-service --amount 0.001 --dry-run --json` checks
   funds and policy without broadcasting.
4. **Execute and verify.** The same transfer with `--wait` returns a successful
   receipt. `tx --chain sepolia <returned-hash> --json` independently looks it up.
   Neither command requests another password.
5. **Enforce the rule.** A 0.006 ETH transfer returns `WHITELIST_DENIED`. The fixture
   checks that the sender nonce did not change.
6. **Account for spending.** The script calculates actual gas from the receipt and
   verifies `0.020 ETH = 0.001 ETH payment + gas + remaining balance`. It reports one
   confirmed payment and one refused request; exact amounts are in the evidence file.
7. **End the session.** `zwallet lock --json` locks the disposable fixture's signer.
   Normal installed binaries lock every signing session for the OS user.
8. **Investment concept — WIP.** The user allocates 50 USDT to an approved mandate.
   A future agent checks assets, venue, quote, slippage, cumulative budget and exposure;
   reports positions, fees and P&L. Returns may be negative. Nothing is traded here.
9. **Shopping concept — WIP.** The user asks for a hardware tool within a 60
   USDT-equivalent ceiling. A future agent compares quotes, includes delivery and
   fees, pays once, and returns order/payment/refund evidence. No order is placed here.
10. **Try it and contribute.** Current CLI primitives are available. Cumulative
    budgets, investment execution, API payments, checkout and native reports are WIP.

## What this checks

The recorder asserts receipt success, the exact recipient balance, unchanged nonce
after policy refusal, the final locked state and the exact allocation equation.
It records the source revision and tool versions alongside the results. This proves
this local path; it does not establish public RPC reliability, merchant fulfillment,
investment performance or an independent security audit.

The [agent integration guide](../../docs/agent-workflow.md) explains how to supply
the same task to your own shell-capable agent and handle pending or failed outcomes.
