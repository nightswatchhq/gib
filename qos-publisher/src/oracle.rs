//! The Gateway QoS Oracle wire format.
//!
//! Field names and types are read off live payloads (`docs/08-qos-publishing.md`), not invented.
//! The consumer's AssemblyScript mapping turns anything it cannot find into `0.0` rather than an
//! error, so a misspelled key here publishes as a confident zero. Rename nothing.

use serde::{Deserialize, Serialize};

/// One record per (deployment, indexer, gateway) per bucket. Topic contains `indexer`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AllocationRecord {
    pub indexer_wallet: String,
    pub indexer_url: String,
    pub subgraph_deployment_ipfs_hash: String,
    /// A chain *name* (`mainnet`, `arbitrum-one`), never a numeric id, despite becoming
    /// `chain_id` in the consuming subgraph.
    pub chain: String,
    pub gateway_id: String,
    /// Unix seconds, not Graph protocol epochs. 300 apart.
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub avg_query_fee: f64,
    pub max_query_fee: f64,
    pub total_query_fees: f64,
    pub query_count: u64,
    pub avg_indexer_latency_ms: f64,
    pub max_indexer_latency_ms: f64,
    pub num_indexer_200_responses: u64,
    pub proportion_indexer_200_responses: f64,
    pub avg_indexer_blocks_behind: f64,
    pub max_indexer_blocks_behind: f64,
    pub stdev_indexer_latency_ms: f64,
}

/// One record per (deployment, gateway) per bucket. Topic contains `query`.
///
/// Note the latency prefix differs from `AllocationRecord` (`gateway_` not `indexer_`), and that
/// there is no `num_*` count field here, only rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRecord {
    pub subgraph_deployment_ipfs_hash: String,
    pub chain: String,
    pub gateway_id: String,
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub total_query_fees: f64,
    pub avg_query_fee: f64,
    pub max_query_fee: f64,
    pub query_count: u64,
    /// Unix **milliseconds**, in the same record as two second-denominated fields.
    pub most_recent_query_ts: i64,
    pub gateway_query_success_rate: f64,
    pub user_attributed_error_rate: f64,
    pub avg_gateway_latency_ms: f64,
    pub max_gateway_latency_ms: f64,
    pub stdev_gateway_latency_ms: f64,
}

/// The topics the public consumer allowlists. Inventing your own means every existing deployment
/// ignores you, so these are the cooperative path.
pub const TOPIC_QUERY: &str = "gateway_query_result_qos_5_minutes_prod_v3";
pub const TOPIC_ALLOCATION: &str = "gateway_indexer_attempt_qos_5_minutes_prod_v3";

/// The on-chain message posted to the Gnosis DataEdge, one per topic per bucket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataEdgeMessage {
    pub topic: String,
    /// CIDv0 of the pinned JSON array.
    pub hash: String,
    /// The bucket's **end**, unix seconds, always a multiple of 300.
    pub timestamp: i64,
}
