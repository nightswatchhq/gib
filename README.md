# Gateway-in-a-Box (gib)

A self-hostable **Graph Protocol subgraph gateway** for The Graph Horizon on Arbitrum One —
in a Docker Compose box. Pour yourself a working, fund-handling gateway without a three-day
setup ceremony.

Under the hood it's the [`lodestar-team/gateway`](https://github.com/lodestar-team/gateway)
fork (MIT, TAP v2 / Horizon-native) plus the `graph-tally` aggregator and escrow-manager, a
Redpanda bus, and optional Prometheus/Grafana — wired together and configured from a single
`.env`. No Edge & Node Studio dependency: you run your own API keys.

```
                      ┌─────────────────────────────────────────────┐
   client ──query──▶  │  gateway  ── signs TAP v2 receipts           │
   (Bearer key)       │     │                                        │
                      │     ├─▶ trusted_indexers ─▶ network subgraph │  (indexer discovery)
                      │     └─▶ selected indexers ─▶ query results    │
                      │                                              │
                      │  redpanda  ◀─ query receipts                 │
                      │     │                                        │
                      │  escrow-manager ─▶ PaymentsEscrow (on-chain) │  (auto top-up)
                      │  tap-aggregator ─▶ RAVs for indexers (public)│
                      └─────────────────────────────────────────────┘
```

## Quickstart

**Prerequisites:** Docker + compose, `gettext-base` (`envsubst`), `python3`, and
[`cast`](https://getfoundry.sh) (Foundry, for key generation). A funded sender wallet is only
needed for real payments ([Stage 2](docs/02-onchain-escrow.md)).

```sh
git clone https://github.com/lodestar-team/gib && cd gib

cp .env.example .env               # 1. fill the TODOs (SENDER_ADDRESS, NETWORK_SUBGRAPH_URL, GATEWAY_API_KEYS)
./scripts/fetch-addresses.sh       # 2. auto-fill verified Horizon addresses -> config/addresses.env
./scripts/gen-keys.sh              # 3. generate sender + signer keys -> ./secrets
./scripts/render.sh                # 4. render runtime/{gateway.json,escrow-manager.json,.env}
docker compose --env-file runtime/.env up -d   # 5. go

docker compose --env-file runtime/.env ps
```

Ships **payment-safe by default**: `PAYMENT_REQUIRED=false` and `ESCROW_DRY_RUN=true`, so
you can validate query routing before a single wei moves. Flip both when you're ready — see
[Stage 2](docs/02-onchain-escrow.md).

### Smoke test

```sh
curl "http://localhost:7700/api/subgraphs/id/<SUBGRAPH_ID>" \
  -H 'content-type: application/json' \
  -H "Authorization: Bearer <one-of-your-GATEWAY_API_KEYS>" \
  -d '{"query":"{ _meta { block { number } } }"}'
```

A block number back = the box discovered indexers, selected up to three, signed receipts and
returned an answer. 🎉

## What's in the box

| Service | Image | Role |
|---------|-------|------|
| `gateway` | `ghcr.io/lodestar-team/gateway` | routes queries, signs TAP v2 receipts |
| `tap-aggregator` | `ghcr.io/graphprotocol/graph_tally_aggregator` | **public** — aggregates receipts → RAVs for indexers |
| `escrow-manager` | `ghcr.io/graphprotocol/graph_tally_escrow_manager` | auto-authorizes signer, tops up escrow |
| `redpanda` | `redpandadata/redpanda` | Kafka-API bus for receipts/attestations |
| `prometheus` + `grafana` | *(optional `monitoring` profile)* | escrow + query dashboards |

## Configure your indexers

- **Allow/block lists & version floors** — work today, no code change (see
  [03 — Operations](docs/03-operations.md)).
- **Selection weights** — gib's patch exposes per-dimension importance exponents (success
  rate / latency / freshness / economic security) in `.env`. Dial latency-vs-stake to your
  taste; defaults reproduce stock gateway behaviour exactly.

## Docs

1. [Keys](docs/01-keys.md) — sender vs signer, generation, handling
2. [On-chain escrow](docs/02-onchain-escrow.md) — going live with real payments
3. [Operations](docs/03-operations.md) — monitoring, indexer selection, network subgraph
4. [Indexer onboarding](docs/04-indexer-onboarding.md) — the aggregator handshake
5. [Upgrades & fork maintenance](docs/05-upgrade-rebase.md) — pinning, rebases

## Status & caveats

- The gateway fork has booted end-to-end against **live Arbitrum One** (~15.8k subgraphs,
  ~182 indexers, signed TAP v2 receipts). Real-payment flow is [Stage 2](docs/02-onchain-escrow.md).
- **Kafka (Redpanda) is required**, not optional — the escrow-manager derives outstanding
  debt from the gateway's exported topics.
- The `tap-aggregator` **must** be internet-reachable by indexers; put TLS in front of it.
- Contract addresses are **new Horizon deployments** — always let `fetch-addresses.sh` pull
  them; never hand-copy.
- Load-test resource sizing (gateway in-memory topology, Redpanda retention) before
  production at Arbitrum One scale.

MIT licensed. A [Lodestar](https://github.com/lodestar-team) project.
