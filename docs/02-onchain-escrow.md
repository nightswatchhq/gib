# 02 — On-chain escrow (going live with real payments)

Until escrow is funded, run with `PAYMENT_REQUIRED=false` and `ESCROW_DRY_RUN=true`: the
gateway routes and answers queries, signs receipts, but no funds move and there are no 402s.
This page is how you flip to real payments.

## The sequence

As a TAP sender on Horizon (Arbitrum One) you must, in order:

1. **Fund the sender** wallet with ETH (gas) + GRT (escrow backing).
2. **Authorize the signer** on the `GraphTallyCollector` — an EIP-712 proof signed by the
   signer key binds it to your sender address.
3. **Approve GRT** to the `PaymentsEscrow` contract.
4. **Deposit into escrow**, per receiver (per indexer).

Steps 2 and 4 are **automated by the escrow-manager** when `ESCROW_DRY_RUN=false` and
`authorize_signers=true` (the default in the rendered config). It reads outstanding query
debt from the `gateway_queries` Kafka topic and sizes deposits up to `GRT_ALLOWANCE`.

## Do it in two stages

### Stage A — dry run (default, no funds move)
```sh
# .env: PAYMENT_REQUIRED=false, ESCROW_DRY_RUN=true
./scripts/render.sh && docker compose --env-file runtime/.env up -d
docker compose --env-file runtime/.env logs -f escrow-manager
```
You'll see the escrow-manager *log* the `authorizeSigner` and deposit txs it **would** send.
Confirm the amounts and the target contracts look right. Route a test query (see the
[README](../README.md#smoke-test)) to prove the pipeline before spending anything.

### Stage B — real payments
```sh
# .env: ESCROW_DRY_RUN=false   (keep PAYMENT_REQUIRED=false one more boot to watch it fund)
./scripts/render.sh && docker compose --env-file runtime/.env up -d escrow-manager
docker compose --env-file runtime/.env logs -f escrow-manager   # watch authorizeSigner + deposit land
```
Once you see escrow funded on-chain (and `escrow_total_balance_grt` climb in Grafana), set
`PAYMENT_REQUIRED=true`, re-render, and restart the gateway. The 402s disappear when your
escrow balance covers issued receipts.

## Contract addresses

Never hand-copy these. `./scripts/fetch-addresses.sh` pulls them from the authoritative
`graphprotocol/contracts` address book (chain 42161) into `config/addresses.env`:

| Field | Contract |
|-------|----------|
| `GRT` | L2GraphToken |
| `GRAPH_TALLY_COLLECTOR` | TAP v2 verifier / signer authorization |
| `PAYMENTS_ESCROW` | Horizon escrow |
| `SUBGRAPH_SERVICE` | Horizon SubgraphService |
| `DISPUTE_MANAGER` | attestation disputes |

Pin a released contracts tag for reproducibility:
```sh
CONTRACTS_REF=v7.1.2 ./scripts/fetch-addresses.sh
```

> ⚠️ Do **not** reuse the legacy TAP v1 escrow `0x8f47…0d3` or any pre-Horizon
> DisputeManager. The auto-fill script only pulls the Horizon deployments.

## The receipt → payment loop (what actually happens)

```
gateway (signer) signs a TAP v2 receipt per indexer request
  → indexer stores receipts
  → indexer's tap-agent asks YOUR tap-aggregator to aggregate them into a RAV
  → aggregator (signer) returns a signed RAV
  → indexer redeems the RAV on-chain against YOUR PaymentsEscrow via GraphTallyCollector
  → escrow-manager watches the drawdown and tops escrow back up
```

> **What's actually on the `gateway_queries` Kafka topic.** Not the signed receipts.
> Each record is a `ClientQueryProtobuf` — `{ receipt_signer, indexer_queries[] }`,
> where each indexer entry is `{ indexer, fee_grt }`. The gateway extracts the fee
> from each signed receipt and **discards the receipt**; the signed receipts exist
> only in-flight, in the `tap-receipt` header sent to the indexer. This is enough for
> billing/metering — the fee amount and the signer (key attribution) are both present,
> which is all the escrow-manager needs to size debt. But any future component that
> needs the *receipts themselves* (e.g. to aggregate them) must obtain them another way
> (mint via the gateway's signer, or capture the header) — they are not in Kafka. The
> `gib smoke` self-test relies on exactly this: it mints receipts through the gateway's
> identical signing path, and separately asserts a live `gateway_queries` record whose
> `receipt_signer` equals the configured signer (the runtime-signer check).

You (the operator) don't redeem RAVs — indexers do. Your job is keeping escrow funded and
the aggregator healthy. See [03 — Operations](03-operations.md).
