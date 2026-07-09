# 03 — Operations

## Network subgraph: how the box discovers indexers

The gateway needs the Graph Network (Arbitrum One) subgraph to discover indexers,
allocations, escrow accounts and authorized signers (deployment
`DZz4kDTdmzWLWsV373w2bSmoar3umKKH9y82SUKr5qmp`). Two ways to serve it:

- **Trusted indexer (recommended for launch).** Point `NETWORK_SUBGRAPH_URL` at a keyed
  public endpoint that serves this subgraph. Zero extra infra. This is what the rendered
  `trusted_indexers` block uses.
- **Self-index (more independence).** Run your own graph-node + IPFS + Postgres and index
  the network subgraph locally, then point `NETWORK_SUBGRAPH_URL` at it. Heavier; out of
  scope for the base box. The compose file leaves room to add these services later.

## Monitoring

```sh
docker compose --profile monitoring up -d
```
- Grafana → `http://127.0.0.1:3000` (admin / `GRAFANA_ADMIN_PASSWORD`). Ships the
  **Escrow & Health** dashboard.
- Prometheus → `http://127.0.0.1:9091`. Scrapes `gateway:7301` and `escrow-manager:9090`.

The signals that matter most:
- `escrow_total_balance_grt` vs `escrow_total_debt_grt` — keep balance comfortably above
  debt. The dashboard's coverage-ratio gauge goes red below 1×.
- Gateway query volume / indexer error rate — watch for a single indexer dominating errors
  (candidate for the blocklist).
- Aggregator health — RAV request failures mean indexers can't get paid; escrow won't draw
  down and indexers may stop serving you.

## Choosing your indexers

Two mechanisms, from simplest to most involved:

### Allow / block lists (works today, no patch needed)
- **Blocklist** — add `(deployment, indexer)` or POI blocks to the gateway config to exclude
  specific indexers or bad data.
- **Version floors** — `MIN_INDEXER_VERSION` / `MIN_GRAPH_NODE_VERSION` in `.env`.
- **Economic pressure** — `QUERY_FEES_TARGET` sets the fee you're willing to pay per query.

### Selection weights (the gib patch)

The query-time selector scores each indexer as a **product of four curves**: success rate,
expected latency, seconds-behind-chain-head, and slashable (economic-security) GRT. gib
exposes a per-dimension **importance exponent** for each — the score becomes
`scoreᵢ ^ weightᵢ`, so a higher weight makes that dimension matter more.

Set any of these in `.env` (blank = `1.0` = stock gateway behaviour, bit-for-bit):

```sh
SELECTION_WEIGHT_SUCCESS_RATE=1.0    # query success rate
SELECTION_WEIGHT_LATENCY=2.0         # ← doubled: favour faster indexers harder
SELECTION_WEIGHT_SECONDS_BEHIND=1.0  # data freshness
SELECTION_WEIGHT_SLASHABLE_GRT=0.5   # ← halved: care less about stake size
```

Re-render and restart the gateway; watch the routing distribution shift in your query
metrics. Example above biases toward low-latency indexers while de-emphasising stake —
the classic "latency-vs-everything-else" dial.

> The deeper curve parameters (logistic midpoints, steepness, exponents) are also
> configurable in the gateway JSON's `selection` block if you need fine control — the
> `.env` exponents are the friendly front for the common case.

> **Not** the same thing as DIPs / indexing-payment selection
> (`subgraph-dips-indexer-selection`), which decides who to *pay to index* a deployment,
> not who to route a live query to. gib only touches query routing.

## Everyday commands

```sh
docker compose --env-file runtime/.env ps
docker compose --env-file runtime/.env logs -f gateway
docker compose --env-file runtime/.env restart gateway     # after a re-render
docker compose --env-file runtime/.env down                # stop (keeps volumes)
```

After changing `.env`, always `./scripts/render.sh` before restarting — the containers read
`runtime/*.json`, not `.env` directly.
