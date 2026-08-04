# 07 — Deploying your own QoS oracle subgraph

The Gateway QoS Oracle is the network's public record of query quality: per
indexer, per subgraph deployment, in 5-minute buckets. Indexers use it to see how
they are performing. Dashboards use it to rank. Nothing else in the protocol
publishes this data.

Today almost everyone reads it through one subgraph deployment maintained by one
person. That is a single point of failure for a feed the whole indexer community
depends on. This document is how you deploy your own copy.

**Everything here has been run.** Build verified from a clean clone; the on-chain
and IPFS facts were read live off Gnosis on 2026-08-04. Where something is
*not* verified, it says so.

## How the feed actually works

```
Gateways ──(5-min aggregates, JSON)──▶ IPFS
         ──({topic, hash, timestamp})──▶ Gnosis DataEdge
                                          0x5b4293b4c0f36cb5d4448950830bc777759b6c4f
                                          submitQoSPayload(bytes)   selector 0x53b73447
                                               │
                                               ▼
                                     your subgraph
                                     callHandler → ipfs.cat → parse → entities
```

The on-chain message is tiny — a JSON object naming a topic and an IPFS hash. The
actual metrics live in the IPFS payload, which the subgraph fetches from inside
the mapping. Two transactions per 5-minute bucket, one per topic, roughly 57
Gnosis blocks apart, about 27,600 gas each.

Note this is **Gnosis**, using a **call handler** on a **non-eventful** DataEdge.
[GRC-002](https://forum.thegraph.com/t/grc-002-qos-oracle-v2/5756) describes a
migration to an eventful DataEdge on Arbitrum One. That migration has not
happened. Build against what is on-chain, not against the GRC.

## Prerequisites

**A Gnosis node with call traces.** This is the one that will cost you a day if
nobody tells you. The subgraph uses `callHandlers`, not event handlers, because
the DataEdge contract emits nothing. Call handlers require the indexing node to
have `trace_filter` — an Erigon or Nethermind archive node with tracing enabled.
A plain RPC endpoint will sync the subgraph to chain head and produce no data at
all, silently.

If you are deploying to Studio or the decentralised network rather than your own
graph-node, check that the indexers serving Gnosis actually support traces before
you assume this works.

**The `ipfsOnEthereumContracts` feature.** Already declared in the manifest.
`ipfs.cat` inside a handler is non-deterministic by nature, so some indexers
decline to index subgraphs that use it. Expect a thin indexer set.

**Tooling.** Node plus the pinned graph-cli. Verified working combination:

| Component | Version |
|---|---|
| Node | 25.8.1 |
| npm | 11.11.0 |
| `@graphprotocol/graph-cli` | 0.98.1 |
| `@graphprotocol/graph-ts` | 0.38.0 |

## Deploy it

The subgraph source is public and MIT-spirited work by Ellipfra:
[`ellipfra/gateway-qos-oracle`](https://github.com/ellipfra/gateway-qos-oracle).
There is also an older
[example subgraph](https://github.com/juanmardefago/gateway-qos-oracle-example-subgraph)
from E&N, which is where the schema originally came from.

```bash
git clone https://github.com/ellipfra/gateway-qos-oracle
cd gateway-qos-oracle
npm install
```

### 1. Whitelist the posters you trust

`src/constants.ts` decides whose payloads become data. Everything else is
recorded as an `OracleMessage` with `valid: false` and produces nothing.

```ts
export let SUBMITTER_WHITELIST: Array<String> = [];
SUBMITTER_WHITELIST.push("0x0b8cef00f90553b9535845be6abbe3797582d424") // legacy E&N poster
SUBMITTER_WHITELIST.push("0x8cbbe43f97f80efa6ba0a95f3d544e03f84db0ce") // current E&N poster
// SUBMITTER_WHITELIST.push("0x...")                                   // your own poster
```

**Addresses must be lowercase.** The mapping compares against
`call.from.toHexString()`, which is lowercase. A checksummed address here never
matches, produces no error, and silently yields an empty subgraph. This is the
single most likely way to lose an afternoon.

If you are publishing your own feed (see
[08 — Publishing your own QoS feed](08-qos-publishing.md)), your poster address
goes here. That is the whole mechanism: **the contract is permissionless, the
subgraph is the filter.** Anyone can call `submitQoSPayload`; each subgraph
decides whose calls count. This is what makes a multi-gateway QoS feed possible
without asking anyone's permission.

### 2. Pick a start block

`subgraph.yaml`:

```yaml
source:
  address: "0x5b4293b4c0f36cb5d4448950830bc777759b6c4f"
  startBlock: 46970110   # Gnosis, 2026-07-01 00:00 UTC
```

The shipped value straddles the E&N poster handover so the subgraph captures
both. Two things to know about going further back:

- There is a genuine **~2.5 day hole** in the feed between 2026-07-01 03:50 UTC
  (block 46972821) and 2026-07-03 14:55 UTC (block 47014403) when nothing was
  posted at all. That is a property of the source data. No subgraph can fill it.
- **Historical payloads do resolve.** Spot-checked on 2026-08-04: a CID from
  2023-11-30 returned 85KB in 0.6s, one from 2025-12-23 returned 2.3MB, one from
  2026-07-03 returned 649KB. E&N pin these properly, so a deep backfill is
  realistic rather than theoretical. Sync time, not availability, is your limit.

### 3. Build

```bash
npx graph codegen
npx graph build
```

Expect one `INFO AS210: Expression is never 'null'` notice from
`src/data-edge.ts:99`. It is cosmetic and present upstream.

### 4. Deploy

```bash
# Studio
npx graph auth <deploy-key>
npx graph deploy <your-subgraph-slug>

# or your own graph-node
npx graph create --node http://localhost:8020/ gateway-qos-oracle
npx graph deploy --node http://localhost:8020/ --ipfs http://localhost:5001 \
  gateway-qos-oracle
```

### 5. Prove it is actually working

Do not trust "synced". Ask whether it parsed anything:

```graphql
{
  oracleMessages(first: 5, orderBy: createdAt, orderDirection: desc) {
    id
    valid
    errorMessage
    createdAt
  }
  messageDataPoints(first: 5, orderBy: timestamp, orderDirection: desc) {
    ipfsHash
    timestamp
    allocationDataPointCount
    queryDataPointCount
  }
}
```

Three distinct failure modes, and they look identical on a status page:

| Symptom | Cause |
|---|---|
| No `oracleMessages` at all | Call handlers not firing — no trace support |
| `oracleMessages` with `valid: false` | Submitter not whitelisted, or not lowercase |
| `valid: true` but `allocationDataPointCount: 0` | `ipfs.cat` returned null; check the IPFS endpoint |

A subgraph that is synced and empty reads as healthy on every dashboard you will
build on top of it. Alert on datapoint counts and on the age of the newest
`MessageDataPoint`, never on sync status alone.

## What you get

| Entity | Grain |
|---|---|
| `OracleMessage` | one per `submitQoSPayload` transaction |
| `MessageDataPoint` | one per IPFS payload referenced by a message |
| `AllocationDataPoint` | 5-min, per (deployment, indexer, gateway) |
| `QueryDataPoint` | 5-min, per (deployment, gateway) |
| `AllocationDailyDataPoint` | daily rollup of the above |
| `IndexerDailyDataPoint` | daily rollup, gateway-wide per indexer |
| `QueryDailyDataPoint` | daily rollup, per deployment |
| `Indexer`, `SubgraphDeployment` | directory entities for navigation |

Every datapoint carries `gateway_id`. The 5-minute entities are immutable and
never pruned; the daily rollups are mutable with `indexerHints: prune: auto`.

Percentiles do not exist at daily grain, and neither does standard deviation for
allocations — a consumer that wants p95 has to work from the 5-minute entities.

## Multiple gateways in one subgraph

Whitelist several posters and every entity keys on `gateway_id`, so the feeds sit
side by side rather than overwriting. That is the format's own design, not a
workaround: E&N's records carry
`gateway_id: 0xff4b7a5efd00ff2ec3518d4f250a27e4c29a2211`, their gateway's
address.

Consumers must then aggregate **per gateway**. Summing `query_count` across
`gateway_id` values without grouping gives you a number that means nothing,
because two gateways may have served the same client query differently.
