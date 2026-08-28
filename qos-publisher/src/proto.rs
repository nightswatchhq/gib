//! The gateway's Kafka wire format, mirrored exactly.
//!
//! These definitions are a copy of `src/reports.rs` in the `nightswatchhq/gateway` fork. The field
//! *tags* are the contract, not the field order or the struct names, so keep the `#[prost(...)]`
//! numbers identical when the gateway changes. Tags 11 and 12 on `ClientQueryProtobuf` are out of
//! sequence in the source too; that is deliberate there and copied faithfully here.

/// One record per client query. Carries its per-indexer attempts nested.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ClientQueryProtobuf {
    /// NOT a gateway identity. The gateway sets this from `graph_env_id`, which gib templates as
    /// `gib-${CHAIN_ID}` — the same value for every gib operator alive. The oracle's `gateway_id`
    /// must be configured separately; see `Config::gateway_id`.
    #[prost(string, tag = "1")]
    pub gateway_id: String,
    /// 20 bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub receipt_signer: Vec<u8>,
    #[prost(string, tag = "3")]
    pub query_id: String,
    #[prost(string, tag = "4")]
    pub api_key: String,
    #[prost(string, tag = "11")]
    pub user_id: String,
    #[prost(string, optional, tag = "12")]
    pub subgraph: Option<String>,
    /// The literal `"success"`, or the error's Display text.
    #[prost(string, tag = "5")]
    pub result: String,
    #[prost(uint32, tag = "6")]
    pub response_time_ms: u32,
    #[prost(uint32, tag = "7")]
    pub request_bytes: u32,
    #[prost(uint32, optional, tag = "8")]
    pub response_bytes: Option<u32>,
    /// USD, derived by the gateway as `total_fees_grt / grt_per_usd`. The oracle wants GRT, and
    /// `grt_per_usd` is not in this message, so GRT is unrecoverable from here: sum the nested
    /// `fee_grt` instead.
    #[prost(double, tag = "9")]
    pub total_fees_usd: f64,
    #[prost(message, repeated, tag = "10")]
    pub indexer_queries: Vec<IndexerQueryProtobuf>,
}

/// One attempt against one indexer.
#[derive(Clone, PartialEq, prost::Message)]
pub struct IndexerQueryProtobuf {
    /// 20 bytes.
    #[prost(bytes = "vec", tag = "1")]
    pub indexer: Vec<u8>,
    /// 32 bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub deployment: Vec<u8>,
    /// 20 bytes.
    #[prost(bytes = "vec", tag = "3")]
    pub allocation: Vec<u8>,
    #[prost(string, tag = "4")]
    pub indexed_chain: String,
    #[prost(string, tag = "5")]
    pub url: String,
    #[prost(double, tag = "6")]
    pub fee_grt: f64,
    #[prost(uint32, tag = "7")]
    pub response_time_ms: u32,
    #[prost(uint32, tag = "8")]
    pub seconds_behind: u32,
    #[prost(string, tag = "9")]
    pub result: String,
    #[prost(string, tag = "10")]
    pub indexer_errors: String,
    /// Marked `// TODO: rm` in the gateway. See `docs/08-qos-publishing.md`.
    #[prost(uint64, tag = "11")]
    pub blocks_behind: u64,
}
