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
                      │  redpanda  ◀─ query fee records (metering)   │
                      │     │                                        │
                      │  escrow-manager ─▶ PaymentsEscrow (on-chain) │  (auto top-up)
                      │  tap-aggregator ─▶ RAVs for indexers (public)│
                      └─────────────────────────────────────────────┘
```

## Requirements

Light. Measured on Arbitrum One with the full network topology resident (~16k
subgraphs, ~26k deployments, ~12.5k indexings):

| Component            | Resident memory |
| -------------------- | --------------- |
| gateway (VmRSS)      | **~207 MB**     |
| redpanda             | ~330 MB         |
| tap-aggregator       | ~10 MB          |
| **full stack**       | **~570 MB**     |

**A 2 GB / 1 vCPU box runs it comfortably.** The gateway holds the network
topology in memory, but that cost is ~200 MB in practice — not the multi-GB you
might expect. (These are measurements, not estimates; supersedes any earlier
2–4 GB guidance.) Disk: a few GB for images + Redpanda retention.

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

### Smoke test — `gib smoke`

The self-test every operator should run **before asking any indexer to whitelist them**. One
command against a running deployment, nothing on-chain, keys unfunded:

```sh
docker compose --profile smoke run --rm smoke
```

It self-configures from your rendered `runtime/gateway.json` and prints a pass/fail table
(nonzero exit on any failure):

| # | Check | What it proves |
|---|-------|----------------|
| a | topology sync | the network subgraph loaded; indexer/subgraph counts within sane bounds |
| b | query dispatched | a real query selected candidate indexers and attached receipts (a `gateway_queries` record appears) |
| c | **runtime signer** | that record's `receipt_signer` equals your configured signer — the *running* gateway is signing with the right key |
| d | mint → aggregate → verify | receipts minted through the gateway's identical signing path aggregate into a RAV whose signature recovers to your signer, with the right EIP-712 domain and `valueAggregate == Σ receipts` |
| e | RAV field assertions | `payer == your sender`, `dataService == your SubgraphService` — config drift fails loudly |
| f | negative tests | a tampered receipt and a wrong-key receipt are both **rejected** by the aggregator |

**Why this isn't circular.** Checks (d–f) mint their own receipts, which alone would only prove
the aggregator works. Check (c) independently confirms the *running* gateway signs with the same
key — observed from a real dispatched query, not minted. Together they cover each other's gap:
(c) proves the live signer, (d–f) prove those signatures aggregate and verify. This mirrors the
original two-stage proof (real indexers recovering the signer from a live query + a verified RAV),
folded into one repeatable command.

**Boundary:** it stops at a verified *signed* RAV. On-chain RAV redemption is the collector
contract's job, not gib's — deliberately untouched.

Prefer a one-liner? A raw client query still works and returns a block number when funded (or a
`402`/`bad indexers` when escrow is empty — which itself proves receipts were signed and
dispatched):

```sh
curl "http://localhost:7700/api/subgraphs/id/<SUBGRAPH_ID>" \
  -H 'content-type: application/json' \
  -H "Authorization: Bearer <one-of-your-GATEWAY_API_KEYS>" \
  -d '{"query":"{ _meta { block { number } } }"}'
```

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

## Getting indexers to accept your gateway

A fresh gib deployment signs valid receipts that **every indexer rejects with a `402`**. This is
protocol design, not a gib defect — and it's the one part of running a gateway you cannot do
alone. Here is exactly what's happening and what to do about it.

### Why paid queries 402 on a fresh deployment

Indexers only serve paid queries from senders they have explicitly whitelisted. The whitelist is
the indexer's `[tap.sender_aggregator_endpoints]` config — a map of `sender address → your
aggregator URL`. It is simultaneously the **trust list** (which senders' receipts the indexer will
accept) and the **aggregator address book** (where the indexer's `tap-agent` sends receipts to be
turned into RAVs). A sender that isn't in the map is rejected, because an unredeemable receipt is a
loss the indexer eats — so serving unknown senders is strictly downside for them.

Concretely, this is the rejection our own deployment received from live Arbitrum indexers (signer
`0x299A…12a3`):

```
402  {"message":"There was an error while accessing escrow account:
             No sender found for signer 0x299A10779ECa64fEBba19839b1AA06c3509D12a3"}
```

The indexer recovered our signer from the receipt, looked it up, found no whitelist entry and no
escrow, and declined. E&N's gateway does not hit this wall because **every indexer already carries
E&N's sender** in their config out of the box. That pre-installed trust is the onboarding moat: it
is protocol-level and social, not technical, and it is the same wall any independent gateway faces.
gib does not remove it — nothing can, short of indexers adding you.

### What you prove alone, before asking anyone for anything

Everything up to the 402 is yours to demonstrate with **zero cooperation** — run
[`gib smoke`](#smoke-test--gib-smoke) and it checks, end to end:

- topology syncs; a real query selects candidate indexers and attaches receipts;
- the **running** gateway signs with your configured signer (observed from its own Kafka record);
- those receipts aggregate into a RAV that recovers to your signer, with the correct EIP-712 domain
  and `valueAggregate == Σ receipts`, and correct `payer` / `dataService`;
- tampered and wrong-key receipts are rejected by the aggregator.

This is the credibility artifact you bring to indexers. The opening line of the ask is literally
*"our sender passes the full `gib smoke` self-test — here's the output."* It says the only thing
left is their whitelist entry and your funded escrow, not any doubt about your stack.

### What requires indexer cooperation (exactly)

**a. The indexer adds one line.** In their indexer config, under the TAP section, they map your
sender address to your public aggregator URL:

```toml
[tap.sender_aggregator_endpoints]
# <your SENDER_ADDRESS> = "<your public aggregator URL>"
0x4a3156cEFBa872eb9711C5f37e52B5118323865C = "https://agg.your-gateway.example"
```

Their `tap-agent` reloads and will now accept your receipts and send them to your aggregator for
RAVs. (Use a stable DNS name for the aggregator, not a bare IP — see
[04 — Indexer onboarding](docs/04-indexer-onboarding.md).)

**b. You fund escrow.** Your sender must be authorized on `GraphTallyCollector` and hold GRT
deposited **per-indexer** in `PaymentsEscrow`, so the indexer's aggregated RAV is actually
redeemable. The escrow-manager automates the authorize + top-up; the full procedure (and the
`PAYMENT_REQUIRED` / `ESCROW_DRY_RUN` flip) is in [02 — On-chain escrow](docs/02-onchain-escrow.md).
Not duplicated here.

**c. If your signers diverge, set `GRAPH_TALLY_PUBLIC_KEYS`.** By default your aggregator only
accepts receipts signed by its own wallet. If you rotate the signer or run more than one gateway
signer, list every signer address in `GRAPH_TALLY_PUBLIC_KEYS` (see `.env.example`) or your own
aggregator will reject those receipts.

### A realistic onboarding sequence

1. **Pick target subgraphs** — the deployments your users actually query.
2. **Identify the indexers serving them with good QoS** — allocations, freshness, low latency (the
   network subgraph and Graph Explorer show who serves what).
3. **Make the ask** to each: your `gib smoke` output, the one `[tap.sender_aggregator_endpoints]`
   line above (your sender = your aggregator URL), and confirmation you've funded per-indexer escrow.
4. **Verify** with a paid query through your gateway to that subgraph — a `200` with data replaces
   the `402`.
5. **Expand** to more subgraphs and indexers.

Honest threshold: the gateway selects **up to three** indexers per query, so **≥3 accepting
indexers per target subgraph** is the practical point at which that subgraph serves reliably. One
or two accepting indexers works but leaves no redundancy.

### Boundary

gib verifies the payment path **up to a signed, verified RAV** — signing, EIP-712 domain,
gateway/aggregator signer consistency, and aggregation, all provable by you alone. What remains is
**on-chain and cooperation-dependent**: the indexer's whitelist entry, escrow funding, and the
indexer redeeming RAVs against your escrow on-chain. Nothing in this repo has moved funds or
redeemed a RAV; no payment has flowed. Those steps are real, they are documented, and they are the
work of onboarding — not of gib.

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
