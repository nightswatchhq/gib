# 04 — Onboarding an indexer

For an indexer to serve your gateway and get paid, it must trust your **tap-aggregator**
endpoint for your **sender** address. This is the handshake — historically the slowest part
of standing up an independent gateway, so make it easy for them.

## Run `gib onboard` first

*(Added 2026-08-28.)* Every failure in this handshake is asymmetric: your aggregator works
perfectly from your own shell, and the indexer discovers hours later that it does not work
from the internet, as receipts that bounce with no diagnosis. That is the wrong way round,
and it is why this step has a reputation.

`gib onboard` checks your side and prints the block below **only if it would actually work**:

```sh
docker compose --profile onboard run --rm onboard \
  --aggregator-url https://aggregator.your-domain.example
```

It refuses when the aggregator URL is loopback, private-range or a Compose service name
(all of which resolve for you and for nobody else), when the aggregator is unreachable or
advertises a different EIP-712 domain than the gateway that signs — a reverse proxy on the
wrong port passes every other check and fails every receipt — or when the collector and
subgraph-service addresses have drifted from `config/addresses.env`.

Plain `http` is a warning, not a refusal. You are asking an indexer to trust that endpoint
with their receipt aggregation; some will decline, and all of them should.

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
