//! Publishes a gateway's own QoS feed to the Gateway QoS Oracle.
//!
//! `docs/08-qos-publishing.md` describes the wire format, read off live payloads and the public
//! consumer's mapping. This crate implements the aggregation half of it: gateway Kafka stream in,
//! the oracle's two 5-minute JSON arrays out.
//!
//! **What is not here yet:** the IPFS pin and the DataEdge transaction. Both need funded keys, and
//! an unpinned payload is a permanent hole in every consumer's history rather than a retryable
//! failure, so neither is worth half-doing. Run `--dry-run` and read the payloads first.

pub mod aggregate;
pub mod attribution;
pub mod cid;
pub mod oracle;
pub mod proto;
