These are synthetic public-address fixtures for the field contract in `.prd/03-tui.md`.
The old handoff did not include its captured HTTP bodies; these files do not claim to be live captures.
Labels and methods are deliberately untrusted dropped fields.

Pending fixture fields follow Blockscout's API v2 renderer: `position` is the transaction
index and `timestamp` is `block_timestamp/1`, which returns null without a block. Pending
`fee.value` can also be null when the node has not supplied gas pricing. These explicit
nulls mean unknown; missing fields, malformed non-null fields and mined nulls are rejected.

- [Transaction renderer and timestamp helper](https://github.com/blockscout/blockscout/blob/master/apps/block_scout_web/lib/block_scout_web/views/api/v2/transaction_view.ex)
- [Transaction fee calculation](https://github.com/blockscout/blockscout/blob/master/apps/explorer/lib/explorer/chain/transaction.ex)

The two explorer pages can describe different moments. Where a transaction row exists,
its state and block metadata take precedence over token-only rows regardless of input order;
token movements remain separate explorer claims until the node detail checks the receipt.
