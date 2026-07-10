# 06 — Topology source (the network subgraph)

The gateway discovers indexers, allocations and deployments from the **Graph
Network subgraph**. It needs a source for it — and this is the one place a fresh
deployment can't be fully self-sufficient, so here's the honest picture and your
options.

## Why you can't just point at a public URL

Two facts collide:

1. **`trusted_indexers` speaks the indexer-service envelope.** The gateway
   expects `{graphQLResponse, attestation}`, not raw GraphQL. A
   decentralised-gateway URL returns bare `{data}` — the wrong shape.
2. **The network subgraph sits behind the same payment wall as every query.**
   Post-Horizon, every real indexer (including `indexer.upgrade.thegraph.com`)
   requires a TAP receipt to serve it — an unauthenticated query gets
   `402 "No Tap receipt was found"`. There is no free, keyless, public endpoint.

E&N's own gateway avoids this only because their upgrade indexer trusts their
gateway internally. A third-party gateway has to bring its own source.

## Default: the bundled topology-adapter (pragmatic, needs a read-only key)

gib ships a small `topology-adapter` service. It fronts The Graph's
**decentralised gateway** with a **read-only Studio API key** and re-wraps the
`{data}` response into the envelope the gateway expects.

```
# .env
TOPOLOGY_STUDIO_KEY=<your read-only key from thegraph.com Studio>
NETWORK_SUBGRAPH_URL=http://gib-topology-adapter:7601/
```

- The key is **read-only**: it queries public topology, signs nothing, holds no
  funds, and never leaves the adapter container's env. It is *not* the same thing
  as your TAP sender/signer keys.
- The adapter's `/health` reports unhealthy until the key is set, and the gateway
  `depends_on` it being healthy — so a missing key fails fast at boot instead of
  looping.
- This is the **pragmatic path while no free public network-subgraph source
  exists**, not an endorsement of a permanent Studio dependency. It keeps you off
  the escrow/whitelist treadmill just to bootstrap topology.

Cost note: the gateway polls the network subgraph roughly every 30s, so a running
gateway consumes Studio query quota continuously. Stop the stack when idle.

## Sovereign alternatives (no key, no adapter)

Both remove the Studio dependency. For either, delete the `topology-adapter`
service and the gateway's `depends_on: topology-adapter` from
`docker-compose.yml`, leave `TOPOLOGY_STUDIO_KEY` blank, and point
`NETWORK_SUBGRAPH_URL` at your own source.

### 1. Self-index the network subgraph

Run a graph-node that indexes the Graph Network Arbitrum subgraph and serves it
through your own indexer-service (which already returns the envelope). Point
`NETWORK_SUBGRAPH_URL` at `http://your-indexer-service:<port>/subgraphs/id/<deployment>`.
Heaviest to operate, but fully sovereign — no external key, no third party.

### 2. A cooperating indexer free-serves you

An indexer you have a relationship with can serve you the network subgraph for
free and issue you a bearer token:

```
NETWORK_SUBGRAPH_URL=https://their-indexer.example/subgraphs/id/<deployment>
NETWORK_SUBGRAPH_AUTH=<free-query token they issue>
```

This is the "trusted indexer" model as originally intended. It needs a real (if
minimal) indexer relationship — the same social step as onboarding, applied to
topology.

## Reference

- Graph Network Arbitrum subgraph id (decentralised-gateway path):
  `DZz4kDTdmzWLWsV373w2bSmoar3umKKH9y82SUKr5qmp`
- Network-subgraph **deployment** id (indexer-service path):
  `QmU5tKrP7YNpm69iUdC3YuQWfJmdye1AHJLLc5pRC4dQBv`
