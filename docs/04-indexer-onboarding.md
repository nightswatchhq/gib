# 04 — Onboarding an indexer

For an indexer to serve your gateway and get paid, it must trust your **tap-aggregator**
endpoint for your **sender** address. This is the handshake — historically the slowest part
of standing up an independent gateway, so make it easy for them.

## What you give the indexer

Two values:

- **Sender address** — `SENDER_ADDRESS` from your `.env`.
- **Aggregator URL** — this box's `:${AGGREGATOR_PORT}` (default 7610), reachable from the
  public internet, ideally behind TLS.

## What the indexer does

Add to their TAP config and restart `indexer-service` + `tap-agent`:

```toml
[tap.sender_aggregator_endpoints]
0xYOUR_SENDER_ADDRESS = "https://aggregator.your-domain.example"
```

That's it. The indexer will now accept your receipts and periodically ask your aggregator to
turn them into RAVs.

## Make the aggregator reachable

The aggregator **must** be internet-facing for indexers, unlike the metrics ports (bound to
localhost). Put it behind a reverse proxy with TLS before sharing the URL widely:

- Terminate TLS at nginx/Caddy/Traefik → proxy to `127.0.0.1:${AGGREGATOR_PORT}`.
- A stable DNS name is kinder to indexers than a bare IP — you don't want to re-onboard
  everyone if the box moves.

## Verify the collector matches

Your `GRAPH_TALLY_COLLECTOR` and `SUBGRAPH_SERVICE` (from `config/addresses.env`) **must**
match what the target indexer uses (`receipts_verifier_address_v2`,
`subgraph_service_address` in their config) or the indexer will reject your receipts. Since
both sides pull from the same Horizon address book, they'll agree on Arbitrum One — but if
an indexer is on a stale config, that's the first thing to check when receipts bounce.

## Sanity check

Once an indexer is onboarded and escrow is funded, route a query that lands on them and
watch `escrow_total_debt_grt` tick up, then `escrow_total_balance_grt` draw down as they
redeem RAVs. If debt rises but balance never falls, the indexer isn't redeeming — check
aggregator reachability and their tap-agent logs.
