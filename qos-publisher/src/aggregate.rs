//! Bucketing and aggregation: gateway query stream -> oracle records.

use std::collections::HashMap;

use crate::attribution::{classify, Blame};
use crate::cid::{address_hex, deployment_to_cid_v0};
use crate::oracle::{AllocationRecord, QueryRecord};
use crate::proto::ClientQueryProtobuf;

/// Bucket width, fixed by the oracle format.
pub const BUCKET_SECS: i64 = 300;

/// The bucket a unix-seconds timestamp belongs to, as `(start, end)`. `end` is what the DataEdge
/// message carries and is always a multiple of 300.
pub fn bucket_bounds(ts_secs: i64) -> (i64, i64) {
    let start = ts_secs.div_euclid(BUCKET_SECS) * BUCKET_SECS;
    (start, start + BUCKET_SECS)
}

/// Population standard deviation. Returns 0.0 for fewer than two samples, matching what the live
/// payloads show for single-sample groups.
fn stdev(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    (samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt()
}

fn mean(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().sum::<f64>() / samples.len() as f64
}

fn max(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(0.0, f64::max)
}

#[derive(Default)]
struct AllocationAcc {
    indexer_wallet: String,
    indexer_url: String,
    deployment_cid: String,
    chain: String,
    latencies: Vec<f64>,
    fees: Vec<f64>,
    blocks_behind: Vec<f64>,
    successes: u64,
}

#[derive(Default)]
struct QueryAcc {
    deployment_cid: String,
    chain: String,
    latencies: Vec<f64>,
    fees: Vec<f64>,
    successes: u64,
    user_errors: u64,
    most_recent_ms: i64,
}

/// One 5-minute window's worth of gateway messages, accumulated.
pub struct Bucket {
    pub start: i64,
    pub end: i64,
    gateway_id: String,
    allocations: HashMap<(String, String), AllocationAcc>,
    queries: HashMap<String, QueryAcc>,
    client_queries: u64,
}

impl Bucket {
    pub fn new(gateway_id: impl Into<String>, ts_secs: i64) -> Self {
        let (start, end) = bucket_bounds(ts_secs);
        Self {
            start,
            end,
            gateway_id: gateway_id.into(),
            allocations: HashMap::new(),
            queries: HashMap::new(),
            client_queries: 0,
        }
    }

    pub fn client_query_count(&self) -> u64 {
        self.client_queries
    }

    /// Folds one client query, and its nested indexer attempts, into this bucket.
    ///
    /// `received_ms` is when the gateway emitted it, in unix milliseconds — the message carries no
    /// timestamp of its own, so the consumer must supply the Kafka record's.
    pub fn add(&mut self, msg: &ClientQueryProtobuf, received_ms: i64) -> anyhow::Result<()> {
        self.client_queries += 1;

        // Deployment attribution: the client record carries a *subgraph*, not a deployment, so the
        // deployment only exists on the nested attempts. During a version migration the attempts
        // can disagree. We attribute the client query to the deployment of its FIRST attempt and
        // accept a small divergence from E&N's numbers rather than silently double-counting.
        let first = msg.indexer_queries.first();

        for attempt in &msg.indexer_queries {
            let cid = deployment_to_cid_v0(&attempt.deployment)?;
            let wallet = address_hex(&attempt.indexer)?;
            let acc = self
                .allocations
                .entry((cid.clone(), wallet.clone()))
                .or_insert_with(|| AllocationAcc {
                    indexer_wallet: wallet,
                    indexer_url: attempt.url.clone(),
                    deployment_cid: cid,
                    chain: attempt.indexed_chain.clone(),
                    ..Default::default()
                });
            acc.latencies.push(attempt.response_time_ms as f64);
            acc.fees.push(attempt.fee_grt);
            acc.blocks_behind.push(attempt.blocks_behind as f64);
            if attempt.result == "success" {
                acc.successes += 1;
            }
        }

        if let Some(first) = first {
            let cid = deployment_to_cid_v0(&first.deployment)?;
            let acc = self.queries.entry(cid.clone()).or_insert_with(|| QueryAcc {
                deployment_cid: cid,
                chain: first.indexed_chain.clone(),
                ..Default::default()
            });
            acc.latencies.push(msg.response_time_ms as f64);
            // GRT, summed from the attempts. `total_fees_usd` on the client record is USD and the
            // exchange rate is not in the message, so it cannot be converted back.
            acc.fees
                .push(msg.indexer_queries.iter().map(|a| a.fee_grt).sum());
            match classify(&msg.result) {
                None => acc.successes += 1,
                Some(Blame::User) => acc.user_errors += 1,
                Some(_) => {}
            }
            acc.most_recent_ms = acc.most_recent_ms.max(received_ms);
        }

        Ok(())
    }

    /// Emits the two JSON arrays for this bucket, as `(allocation_records, query_records)`.
    pub fn close(&self) -> (Vec<AllocationRecord>, Vec<QueryRecord>) {
        let allocations = self
            .allocations
            .values()
            .map(|a| {
                let count = a.latencies.len() as u64;
                AllocationRecord {
                    indexer_wallet: a.indexer_wallet.clone(),
                    indexer_url: a.indexer_url.clone(),
                    subgraph_deployment_ipfs_hash: a.deployment_cid.clone(),
                    chain: a.chain.clone(),
                    gateway_id: self.gateway_id.clone(),
                    start_epoch: self.start,
                    end_epoch: self.end,
                    avg_query_fee: mean(&a.fees),
                    max_query_fee: max(&a.fees),
                    total_query_fees: a.fees.iter().sum(),
                    query_count: count,
                    avg_indexer_latency_ms: mean(&a.latencies),
                    max_indexer_latency_ms: max(&a.latencies),
                    num_indexer_200_responses: a.successes,
                    proportion_indexer_200_responses: if count == 0 {
                        0.0
                    } else {
                        a.successes as f64 / count as f64
                    },
                    avg_indexer_blocks_behind: mean(&a.blocks_behind),
                    max_indexer_blocks_behind: max(&a.blocks_behind),
                    stdev_indexer_latency_ms: stdev(&a.latencies),
                }
            })
            .collect();

        let queries = self
            .queries
            .values()
            .map(|q| {
                let count = q.latencies.len() as u64;
                let n = count as f64;
                QueryRecord {
                    subgraph_deployment_ipfs_hash: q.deployment_cid.clone(),
                    chain: q.chain.clone(),
                    gateway_id: self.gateway_id.clone(),
                    start_epoch: self.start,
                    end_epoch: self.end,
                    total_query_fees: q.fees.iter().sum(),
                    avg_query_fee: mean(&q.fees),
                    max_query_fee: max(&q.fees),
                    query_count: count,
                    most_recent_query_ts: q.most_recent_ms,
                    gateway_query_success_rate: if count == 0 {
                        0.0
                    } else {
                        q.successes as f64 / n
                    },
                    user_attributed_error_rate: if count == 0 {
                        0.0
                    } else {
                        q.user_errors as f64 / n
                    },
                    avg_gateway_latency_ms: mean(&q.latencies),
                    max_gateway_latency_ms: max(&q.latencies),
                    stdev_gateway_latency_ms: stdev(&q.latencies),
                }
            })
            .collect();

        (allocations, queries)
    }
}
