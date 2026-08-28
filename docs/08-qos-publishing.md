# 08 — Publishing your own QoS feed

[07](07-qos-subgraph.md) covers reading the QoS oracle. This one covers writing to
it: how a gateway that is not Edge & Node publishes its own quality data under its
own `gateway_id`.

## Status: the aggregation half is built; nothing has been published

**Updated 2026-08-28.** gib now ships `qos-publisher/`, a Rust crate implementing the
aggregation described below: gateway Kafka stream in, the oracle's two 5-minute
JSON arrays out, with the bucketing, the statistics, the CIDv0 encoding and the
error attribution under test. Run it with `--dry-run` and read the payloads.

**What still has not happened: nothing has ever been pinned or posted.** The IPFS
pin and the DataEdge transaction are deliberately not implemented. Both need
funded keys, and an unpinned payload is a *permanent hole in every consumer's
history* rather than a retryable failure, so neither is worth half-doing. The
binary refuses to run without `--dry-run` rather than pretending otherwise.

No component of the publishing side exists as open source anywhere that we could
find — not in `edgeandnode`, not in `graphprotocol`. E&N's aggregator is internal.

What follows is the wire format, read off two sources that cannot lie about it:

1. the public consumer,
   [`ellipfra/gateway-qos-oracle`](https://github.com/ellipfra/gateway-qos-oracle),
   whose AssemblyScript mapping is a complete parser for the format; and
2. live payloads pulled off Gnosis and IPFS on 2026-08-04 and read field by field.

So the format below is **observed**, not guessed. The mapping from gib's Kafka
stream onto it is **reasoned**, and marked where it is uncertain. Treat this as a
build spec that someone still has to build, and correct it against reality when
they do.

The upside: gib already produces the data. The gateway's `gateway_queries` topic
carries every field the format needs, and gib already runs the Redpanda that holds
it. The missing piece is an aggregator, an IPFS pin and one transaction.

## The format

### On-chain message

Call `submitQoSPayload(bytes)` on the Gnosis DataEdge at
`0x5b4293b4c0f36cb5d4448950830bc777759b6c4f` (selector `0x53b73447`). The bytes
are UTF-8 JSON — either one object or an array of them:

```json
{"topic": "gateway_query_result_qos_5_minutes_prod_v3",
 "hash": "QmSKLoV5PnX9tpwGkq7R1uWm7NWLPp3vrJvKSj2gdrR6zv",
 "timestamp": 1785799800}
```

`timestamp` is the bucket's **end**, unix seconds, always a multiple of 300.

Observed E&N behaviour: one transaction per topic per bucket, two per bucket
total, both landing in the same block, ~27,600 gas each. On Gnosis that is
rounding error.

The contract takes calls from anyone. Whether your payload becomes data is
decided by the consuming subgraph's whitelist, not by the chain.

### Topics

The consumer only processes topics on its allowlist
(`src/constants.ts`, `JSON_TOPICS`):

```
gateway_query_result_qos_5_minutes_prod_v2
gateway_indexer_attempt_qos_5_minutes_prod_v2
gateway_query_result_qos_5_minutes_prod_v3
gateway_indexer_attempt_qos_5_minutes_prod_v3
```

Routing is by substring: a topic containing `indexer` is parsed as allocation
records, one containing `query` as query records. `v3` is current.

**If you invent your own topic string, every existing deployment ignores you.**
You have a choice, and it is a real one:

- **Reuse the `prod_v3` topics.** Any subgraph that whitelists your poster picks
  you up with no code change. Your records are distinguished by `gateway_id`.
  This is the cooperative path and the one we would take.
- **Mint your own topic.** Total isolation; requires every consumer to add it to
  `JSON_TOPICS` and redeploy. Only worth it if your schema diverges.

### IPFS payload

A JSON array of flat objects, pinned to IPFS, CIDv0. One array per topic per
bucket. Observed size: 2,439 allocation records in a single 5-minute bucket, about
650KB to 2.3MB per payload.

**Allocation records** (`gateway_indexer_attempt_...`), one per
(deployment, indexer, gateway) per bucket. Real record, unedited:

```json
{
  "indexer_wallet": "0xf92f430dd8567b0d466358c79594ab58d919a6d4",
  "indexer_url": "https://graph-l2prod.ellipfra.com/",
  "subgraph_deployment_ipfs_hash": "QmRQQTgkdtY3mxjGfbbrSJfDbBB8FAE8jSsNfPfk7SfzCN",
  "chain": "arbitrum-one",
  "gateway_id": "0xff4b7a5efd00ff2ec3518d4f250a27e4c29a2211",
  "start_epoch": 1785799200,
  "end_epoch": 1785799500,
  "avg_query_fee": 0.0006383103,
  "max_query_fee": 0.0006623695,
  "total_query_fees": 0.0497882016,
  "query_count": 78,
  "avg_indexer_latency_ms": 266.0512820513,
  "max_indexer_latency_ms": 838,
  "num_indexer_200_responses": 78,
  "proportion_indexer_200_responses": 1.0,
  "avg_indexer_blocks_behind": 15.9487179487,
  "max_indexer_blocks_behind": 142,
  "stdev_indexer_latency_ms": 153.2520966519
}
```

**Query records** (`gateway_query_result_...`), one per (deployment, gateway) per
bucket:

```json
{
  "subgraph_deployment_ipfs_hash": "QmZk7ThfQVkwhwckCPtqXmxC8SRD99gLeF9pxfYfEiVwV1",
  "chain": "mainnet",
  "gateway_id": "0xff4b7a5efd00ff2ec3518d4f250a27e4c29a2211",
  "start_epoch": 1785799500,
  "end_epoch": 1785799800,
  "total_query_fees": 11.9019417079,
  "avg_query_fee": 0.0013482036,
  "max_query_fee": 0.0013483176,
  "query_count": 8828,
  "most_recent_query_ts": 1785799799992,
  "gateway_query_success_rate": 1.0,
  "user_attributed_error_rate": 0.0,
  "avg_gateway_latency_ms": 186.6964204803,
  "max_gateway_latency_ms": 5008,
  "stdev_gateway_latency_ms": 303.7156840987
}
```

### Traps in the format

Each of these is a real inconsistency in live data, not pedantry.

- **`start_epoch` and `end_epoch` are unix seconds**, despite the name. They are
  not Graph protocol epochs. 300 apart.
- **`most_recent_query_ts` is unix milliseconds** — in the same record as two
  second-denominated fields. Read it wrong and your freshness metric is off by a
  factor of 1000.
- **`chain` on the wire becomes `chain_id` in the subgraph.** It holds a chain
  *name* (`mainnet`, `arbitrum-one`, `avalanche`), never a numeric id.
- **`gateway_id` is a free-form string.** E&N put their gateway's address in it.
  Lodestar uses the literal `lodestar`. Nothing enforces either. Pick one, make it
  stable forever, and be aware consumers may treat it as an address.
- **Do NOT take `gateway_id` from the protobuf.** *(Correction, 2026-08-28.)* The
  Kafka message has a field of that name and it is not a gateway identity: the
  gateway fills it from `graph_env_id`, which gib templates as `gib-${CHAIN_ID}`
  (see `config/gateway.json.tmpl`). Map it straight through and every gib operator
  alive publishes under `gib-42161`, into one indistinguishable bucket. The
  publisher takes `--gateway-id` separately and ignores the protobuf field.
- **Fees are GRT as JSON floats**, not wei, not USD.
- **The two topics use different latency prefixes**: `*_indexer_latency_ms` on
  allocation records, `*_gateway_latency_ms` on query records. Query records also
  have no `num_*` count field, only rates.
- Anything the parser cannot find becomes zero, not an error. A misspelled key
  publishes as a confident `0.0`.

## Mapping gib's Kafka stream onto the format

The gateway writes protobuf to `gateway_queries` (see `src/reports.rs` in the
gateway fork). One message per client query, carrying nested per-indexer attempts:

```
ClientQueryProtobuf { gateway_id, query_id, result, response_time_ms,
                      total_fees_usd, subgraph, indexer_queries[] }
  └─ IndexerQueryProtobuf { indexer, deployment, allocation, indexed_chain, url,
                            fee_grt, response_time_ms, seconds_behind,
                            blocks_behind, result, indexer_errors }
```

`result` is the literal string `"success"` or the error's Display text.

### Allocation records ← `indexer_queries[]`

Group by (deployment, indexer, bucket).

| Oracle field | Source |
|---|---|
| `indexer_wallet` | `indexer` (20 bytes) as lowercase `0x` hex |
| `indexer_url` | `url` |
| `subgraph_deployment_ipfs_hash` | `deployment` (32 bytes) → CIDv0 base58, prefix `0x1220` |
| `chain` | `indexed_chain` |
| `gateway_id` | yours |
| `start_epoch` / `end_epoch` | bucket bounds |
| `query_count` | count of attempts |
| `num_indexer_200_responses` | attempts where `result == "success"` |
| `proportion_indexer_200_responses` | the above ÷ `query_count` |
| `avg`/`max`/`stdev_indexer_latency_ms` | `response_time_ms` |
| `avg`/`max_indexer_blocks_behind` | `blocks_behind` — see below |
| `avg`/`max_query_fee`, `total_query_fees` | `fee_grt` |

**`blocks_behind` is marked `// TODO: rm` in the gateway source.** The gateway is
moving to `seconds_behind` as the real signal. When that field goes, you either
derive blocks from `seconds_behind` divided by the chain's block time — an
approximation the oracle schema cannot express as one — or you publish zero and
quietly corrupt every consumer's freshness ranking. Decide deliberately.

### Query records ← `ClientQueryProtobuf`

Group by (deployment, bucket).

| Oracle field | Source |
|---|---|
| `avg`/`max`/`stdev_gateway_latency_ms` | client `response_time_ms` |
| `query_count` | count of client queries |
| `gateway_query_success_rate` | fraction with `result == "success"` |
| `user_attributed_error_rate` | fraction whose error is the client's fault |
| `most_recent_query_ts` | latest query timestamp, **milliseconds** |
| `total`/`avg`/`max_query_fee` | sum of nested `fee_grt` — see below |
| `chain` | from the attempts |

Two problems here, both unresolved:

**Deployment attribution.** The client record carries `subgraph`, not a
deployment; the deployment only exists on the nested attempts. Usually every
attempt for one query targets the same deployment and it does not matter. During
a subgraph version migration they can differ, and the format has no way to say
"this query touched two deployments". E&N must resolve this somehow. We do not
know how. Pick a rule, write it down, and expect a small discrepancy against
their numbers.

**Fees.** `total_fees_usd` on the client record is USD; the oracle wants GRT. Sum
the nested `fee_grt` instead. That sums only *attempted* indexer fees, which is
subtly different from what the client was charged.

**`user_attributed_error_rate`** needs a classification the gateway does not
publish directly. From `src/errors.rs`, the sensible split is:

| Prefix | Attribution |
|---|---|
| `auth error:` | user |
| `bad query:` | user |
| `subgraph not found:` | user |
| `no indexers found` | network |
| `bad indexers:` | network |
| `internal error:` | you |

Whether E&N draw the line in the same place is unverified. If your figure differs
from theirs on the same traffic, this is the first place to look.

## What you would have to build

Steps 1 to 3 are now built in `qos-publisher/`; 4 to 6 are not. A single service,
and gib already hands it the hard part:

1. **Consume** `gateway_queries` from the Redpanda gib already runs.
2. **Bucket** into 5-minute windows aligned to multiples of 300 unix seconds.
   Hold open buckets in Postgres, not memory — a restart mid-bucket should not
   lose a window.
3. **Close** each bucket on a delay for late messages, then emit two JSON arrays.
4. **Pin** each array to IPFS and keep it pinned. Consumers refetch during
   resync, sometimes years later; an unpinned payload is a permanent hole in
   everyone's history, not just yours.
5. **Post** `{topic, hash, timestamp}` to the DataEdge from a dedicated poster
   key holding a few xDAI. It signs nothing else and holds no funds.
6. **Get whitelisted** in whichever subgraphs should carry you, starting with
   your own from [07](07-qos-subgraph.md).

Roughly: `rdkafka` in, Postgres in the middle, an IPFS client and `alloy` out.
GRC-002 describes E&N's build in the same shape, which is mild evidence the
shape is right.

Two operational notes worth having before you start:

- **Your poster must not stall silently.** When E&N's stalled for ~38 hours, the
  tell was not a failed transaction — the wallet was funded, the nonces were
  contiguous, nothing errored. The tell was publish *lag* drifting from 30 to 48
  minutes about 17 minutes before it died, and then dying between the two topics
  of a single bucket. Alert on the gap between newest bucket and wall clock, and
  alert on the two topics disagreeing about which bucket is newest. It also
  resumed from tip without backfilling, which is why the hole is permanent.
- **Publishing nothing looks exactly like publishing perfection.** A gateway with
  no traffic emits no records, and every consumer renders that as blank rather
  than as absent. If you publish, publish your own liveness too.

## Why bother

Because the QoS feed is the only public record of indexer quality, and today it
has one producer and effectively one consumer deployment. A second gateway
publishing under its own `gateway_id` makes the feed what its schema always
assumed it was: multi-gateway. Indexers get a second opinion on their own
performance, from a party with no stake in flattering them.

That is not a hypothetical benefit. It is the difference between a network metric
and one company's telemetry.
