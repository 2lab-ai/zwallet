# Explorer re-measure, 2026-10-28

The receipt behind 03-tui.md §History "Re-measured 2026-10-28". The raw response bodies were kept in the dispatcher's session scratchpad only (about 1.6 MB of JSON); this file keeps every status line and the decision.


Blockscout `GET /api/v2/addresses/0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045/transactions`:
| Host | HTTP | Content-Type | Result |
|---|---|---|---|
| eth.blockscout.com | 200 | application/json | 50 items, 531,134 bytes |
| base.blockscout.com | 403 | text/html | Cloudflare "Just a moment..." |
| arbitrum.blockscout.com | 403 | text/html | Cloudflare "Just a moment..." |
| explorer.optimism.io | 200 | application/json | 50 items, 527,444 bytes |
| polygon.blockscout.com | 403 | text/html | Cloudflare "Just a moment..." |
| bsc.blockscout.com | 404 | text/plain | "default backend - 404" |
| avalanche.blockscout.com | 404 | text/plain | "default backend - 404" |
| eth-sepolia.blockscout.com | 200 | application/json | 50 items, 498,828 bytes |

Routescan, both paths, chain ids 8453 (base), 42161 (arbitrum), 137 (polygon), 56 (bsc), 43114 (avalanche):
- v2 `/v2/network/mainnet/evm/<id>/address/<a>/transactions?limit=10`: 8453, 42161, 56 → 400 `NO_BLOCKCHAINS_FOR_PARAMS`; 137 → 400 `BLOCKCHAIN_NOTFOUND`; 43114 → 200, 10 items.
- Etherscan-style `/v2/network/mainnet/evm/<id>/etherscan/api?module=account&action=txlist&address=<a>&page=1&offset=10&sort=desc`: 8453, 42161, 137, 56 → 200 `{"status":"0","message":"chain not supported","result":null}`; 43114 → 200, 10 items.

Conclusion: same as 2026-10-27. No keyless explorer path opens base, arbitrum, polygon or bsc; Routescan answers only avalanche (both paths). Raw bodies: not kept in the repo (size).
